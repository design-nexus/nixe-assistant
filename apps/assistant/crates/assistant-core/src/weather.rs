//! Direct, bounded weather retrieval; search snippets are never current conditions.
use crate::{location::LocationService, Config};
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::time::Duration;

pub fn is_weather(ask: &str) -> bool {
    let s = ask.to_lowercase();
    s.contains("weather")
        || s.contains("forecast")
        || s.contains("temperature outside")
        || s.contains("hot outside")
        || s.contains("cold outside")
}
/// Only explicit place suffixes; relative places and time/unit phrases use configured location.
pub fn place(ask: &str) -> Option<String> {
    let lower = ask.to_ascii_lowercase();
    if !crate::location::explicit_place(ask) {
        return None;
    }
    let start = lower.find(" in ").or_else(|| lower.find(" for "))? + 4;
    let raw = &ask[start..];
    let mut end = raw.len();
    let tail = raw.to_ascii_lowercase();
    for suffix in [
        " today",
        " tomorrow",
        " right now",
        " this week",
        " next week",
        " in celsius",
        " in fahrenheit",
        " please",
    ] {
        if let Some(index) = tail.find(suffix) {
            end = end.min(index);
        }
    }
    let city = raw[..end].trim().trim_end_matches(['?', '.', '!']).trim();
    (!city.is_empty()).then(|| city.to_string())
}
async fn get(client: &reqwest::Client, url: &str, query: &[(&str, String)]) -> Result<Value> {
    let mut response = client
        .get(url)
        .query(query)
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= 65536,
            "weather response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes)?;
    anyhow::ensure!(
        value["error"] != true,
        "weather provider rejected the request"
    );
    Ok(value)
}
fn coordinates(v: &Value) -> Result<(f64, f64)> {
    let lat = v["latitude"].as_f64().context("missing weather latitude")?;
    let lon = v["longitude"]
        .as_f64()
        .context("missing weather longitude")?;
    anyhow::ensure!(
        lat.is_finite()
            && lon.is_finite()
            && (-90.0..=90.0).contains(&lat)
            && (-180.0..=180.0).contains(&lon),
        "invalid weather coordinates"
    );
    Ok((lat, lon))
}
pub async fn lookup(
    service: &LocationService,
    cfg: &Config,
    city: Option<&str>,
    celsius: bool,
) -> Result<Value> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(5))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut label;
    let mut coords = None;
    let mut approximate = false;
    if let Some(city) = city.filter(|s| !s.trim().is_empty()) {
        label = city.trim().to_string();
    } else if cfg.location.enabled && !cfg.browser.weather_location.is_empty() {
        label = cfg.browser.weather_location.clone();
    } else {
        let location = service.resolve(&cfg.location, false).await?;
        label = location.label;
        approximate = location.approximate;
        coords = location.latitude.zip(location.longitude);
    }
    if coords.is_none() {
        let mut components = label.split(',').map(str::trim);
        let name = components.next().context("missing weather city")?;
        let region = components.next().unwrap_or("").to_lowercase();
        let value = get(
            &client,
            "https://geocoding-api.open-meteo.com/v1/search",
            &[
                ("name", name.into()),
                ("count", "10".into()),
                ("language", "en".into()),
                ("format", "json".into()),
            ],
        )
        .await?;
        let results = value["results"]
            .as_array()
            .context("no matching weather city; specify a city and region")?;
        let found = if region.is_empty() {
            results.first()
        } else {
            results.iter().find(|v| {
                ["admin1", "country", "country_code"]
                    .iter()
                    .any(|key| v[key].as_str().is_some_and(|s| s.to_lowercase() == region))
            })
        }
        .context("no matching weather city and region")?;
        coords = Some(coordinates(found)?);
        label = ["name", "admin1", "country"]
            .iter()
            .filter_map(|k| found[k].as_str())
            .collect::<Vec<_>>()
            .join(", ");
    }
    // US locations always use imperial units, including tool-generated requests.
    let celsius = celsius && !is_us(&label);
    let (lat, lon) = coords.context("missing weather coordinates")?;
    let data = get(&client, "https://api.open-meteo.com/v1/forecast", &[
        ("latitude",lat.to_string()),("longitude",lon.to_string()),
        ("current","temperature_2m,apparent_temperature,relative_humidity_2m,weather_code,wind_speed_10m".into()),
        ("daily","weather_code,temperature_2m_max,temperature_2m_min,precipitation_probability_max".into()),
        ("forecast_days","7".into()),("timezone","auto".into()),
        ("temperature_unit",if celsius {"celsius"} else {"fahrenheit"}.into()),
        ("wind_speed_unit",if celsius {"kmh"} else {"mph"}.into()),
    ]).await?;
    validate_current(&data)?;
    validate_fresh(&data)?;
    Ok(
        json!({"source":"Open-Meteo","source_url":"https://open-meteo.com/","location":label,"approximate_location":approximate,"timezone":data["timezone"],"current":data["current"],"current_units":data["current_units"],"daily":data["daily"],"daily_units":data["daily_units"],"condition":condition(data["current"]["weather_code"].as_u64().unwrap_or(999)),"retrieved_at":chrono::Utc::now().to_rfc3339(),"note":"Current conditions are weather-model estimates, not a station observation."}),
    )
}
fn validate_fresh(data: &Value) -> Result<()> {
    let time = data["current"]["time"]
        .as_str()
        .context("missing weather timestamp")?;
    let local = chrono::NaiveDateTime::parse_from_str(time, "%Y-%m-%dT%H:%M")?;
    let offset = data["utc_offset_seconds"]
        .as_i64()
        .context("missing weather timezone offset")?;
    anyhow::ensure!(
        (-86400..=86400).contains(&offset),
        "invalid weather timezone offset"
    );
    let utc = local.and_utc() - chrono::Duration::seconds(offset);
    let age = chrono::Utc::now().signed_duration_since(utc).num_seconds();
    anyhow::ensure!(
        (-1800..=7200).contains(&age),
        "weather provider returned stale or future conditions"
    );
    Ok(())
}
fn validate_current(data: &Value) -> Result<()> {
    anyhow::ensure!(
        data["current"]["time"].as_str().is_some()
            && data["current"]["temperature_2m"].as_f64().is_some()
            && data["current"]["weather_code"].as_u64().is_some()
            && data["current_units"]["temperature_2m"].as_str().is_some(),
        "weather provider returned incomplete current conditions"
    );
    Ok(())
}
fn is_us(label: &str) -> bool {
    let label = label.to_ascii_lowercase();
    label.contains("united states")
        || label
            .split(',')
            .any(|part| matches!(part.trim(), "us" | "usa"))
}
pub fn condition(code: u64) -> &'static str {
    match code {
        0 => "clear",
        1 => "mostly clear",
        2 => "partly cloudy",
        3 => "overcast",
        45 | 48 => "foggy",
        51 | 53 | 55 => "drizzle",
        56 | 57 => "freezing drizzle",
        61 | 63 | 65 => "rain",
        66 | 67 => "freezing rain",
        71 | 73 | 75 | 77 => "snow",
        80..=82 => "rain showers",
        85 | 86 => "snow showers",
        95..=99 => "thunderstorms",
        _ => "conditions unavailable",
    }
}
pub fn current_reply(v: &Value) -> Result<String> {
    validate_current(v)?;
    let current = &v["current"];
    let unit = if v["current_units"]["temperature_2m"] == "°C" {
        "Celsius"
    } else {
        "Fahrenheit"
    };
    let label = v["location"].as_str().context("missing weather location")?;
    let time = current["time"].as_str().unwrap();
    let temp = current["temperature_2m"].as_f64().unwrap();
    let condition = condition(current["weather_code"].as_u64().unwrap());
    let feels = current["apparent_temperature"]
        .as_f64()
        .map(|n| format!(", feeling like {n:.0} degrees"))
        .unwrap_or_default();
    Ok(format!("For {label}, Open-Meteo reports {condition} and {temp:.0} degrees {unit}{feels}, as of {} local time.",time.split('T').nth(1).unwrap_or(time)))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn missing_conditions_and_explicit_places() {
        assert!(is_us("Chicago, Illinois, United States"));
        assert!(!is_us("Paris, France"));
        assert!(validate_fresh(
            &json!({"current":{"time":"2000-01-01T00:00"},"utc_offset_seconds":0})
        )
        .is_err());
        assert!(validate_current(&json!({"current":{"temperature_2m":null}})).is_err());
        assert_eq!(
            place("What's the weather in Chicago, Illinois today?"),
            Some("Chicago, Illinois".into())
        );
        assert_eq!(place("weather in my area"), None);
        assert_eq!(place("weather in Celsius"), None);
        let data = json!({"location":"City","current":{"time":"2026-10-07T12:15","temperature_2m":79.1,"apparent_temperature":80.3,"weather_code":0},"current_units":{"temperature_2m":"°F"}});
        let text = current_reply(&data).unwrap();
        assert!(text.contains("79 degrees Fahrenheit") && text.contains("12:15 local time"));
    }
}
