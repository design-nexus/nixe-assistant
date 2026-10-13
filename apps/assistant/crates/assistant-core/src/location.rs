//! On-demand city-level location. Never persist IP lookup responses or infer GPS precision.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::{Duration, Instant};
use tokio::sync::Mutex;
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct LocationCfg {
    pub enabled: bool,
    pub city: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub cache_minutes: u64,
}
impl Default for LocationCfg {
    fn default() -> Self {
        Self {
            enabled: true,
            city: String::new(),
            latitude: None,
            longitude: None,
            cache_minutes: 60,
        }
    }
}
impl LocationCfg {
    pub fn validate(&self) -> Result<()> {
        anyhow::ensure!(
            self.city.len() <= 256 && !self.city.chars().any(char::is_control),
            "invalid location name"
        );
        anyhow::ensure!(
            (1..=1440).contains(&self.cache_minutes),
            "location cache must be 1–1440 minutes"
        );
        match (self.latitude, self.longitude) {
            (None, None) => {}
            (Some(lat), Some(lon)) => anyhow::ensure!(
                lat.is_finite()
                    && lon.is_finite()
                    && (-90.0..=90.0).contains(&lat)
                    && (-180.0..=180.0).contains(&lon)
                    && !self.city.trim().is_empty(),
                "invalid location coordinates or missing city"
            ),
            _ => anyhow::bail!("provide both latitude and longitude"),
        };
        Ok(())
    }
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Location {
    pub label: String,
    pub latitude: Option<f64>,
    pub longitude: Option<f64>,
    pub timezone: Option<String>,
    pub source: String,
    pub approximate: bool,
    pub detected_at: String,
}
#[derive(Default)]
pub struct LocationService {
    cache: Mutex<Option<(Instant, LocationCfg, Location)>>,
}
impl LocationService {
    pub async fn clear(&self) {
        *self.cache.lock().await = None;
    }
    pub async fn resolve(&self, cfg: &LocationCfg, refresh: bool) -> Result<Location> {
        cfg.validate()?;
        let mut cache = self.cache.lock().await;
        if !cfg.enabled {
            *cache = None;
            anyhow::bail!("location is disabled");
        }
        if !cfg.city.trim().is_empty() {
            return Ok(Location {
                label: cfg.city.trim().into(),
                latitude: cfg.latitude,
                longitude: cfg.longitude,
                timezone: None,
                source: "saved".into(),
                approximate: cfg.latitude.is_none(),
                detected_at: chrono::Utc::now().to_rfc3339(),
            });
        }
        if !refresh {
            if let Some((when, settings, location)) = &*cache {
                if settings == cfg && when.elapsed() < Duration::from_secs(cfg.cache_minutes * 60) {
                    return Ok(location.clone());
                }
            }
        }
        // Failed refreshes never silently return an expired location.
        *cache = None;
        let location = match fetch_ip("https://ipapi.co/json/", "ipapi.co").await {
            Ok(location) => location,
            Err(primary) => fetch_ip(
                "https://ipwho.is/?fields=success,city,region,country,latitude,longitude,timezone.id",
                "ipwho.is",
            )
            .await
            .with_context(|| format!("primary location lookup failed: {primary}"))?,
        };
        *cache = Some((Instant::now(), cfg.clone(), location.clone()));
        Ok(location)
    }
}
async fn fetch_ip(url: &str, source: &str) -> Result<Location> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(3))
        .redirect(reqwest::redirect::Policy::none())
        .build()?;
    let mut response = client.get(url).send().await?.error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        anyhow::ensure!(
            bytes.len() + chunk.len() <= 16384,
            "location response too large"
        );
        bytes.extend_from_slice(&chunk);
    }
    decode_provider(&serde_json::from_slice(&bytes)?, source)
}
fn decode_provider(v: &Value, source: &str) -> Result<Location> {
    if source == "ipwho.is" {
        anyhow::ensure!(
            v["success"] == true,
            "location provider could not resolve this network"
        );
        let mut normalized = v.clone();
        normalized["country_name"] = v["country"].clone();
        normalized["timezone"] = v["timezone"]["id"].clone();
        let mut location = decode_ip(&normalized)?;
        location.source = source.into();
        Ok(location)
    } else {
        decode_ip(v)
    }
}
fn decode_ip(v: &Value) -> Result<Location> {
    anyhow::ensure!(
        v["error"] != true,
        "location provider could not resolve this network"
    );
    let city = v["city"]
        .as_str()
        .filter(|s| !s.trim().is_empty())
        .context("location provider returned no city")?;
    let label = [Some(city), v["region"].as_str(), v["country_name"].as_str()]
        .into_iter()
        .flatten()
        .filter(|s| !s.trim().is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    let latitude = v["latitude"]
        .as_f64()
        .context("location provider returned no latitude")?;
    let longitude = v["longitude"]
        .as_f64()
        .context("location provider returned no longitude")?;
    LocationCfg {
        city: label.clone(),
        latitude: Some(latitude),
        longitude: Some(longitude),
        ..Default::default()
    }
    .validate()?;
    Ok(Location {
        label,
        latitude: Some(latitude),
        longitude: Some(longitude),
        timezone: v["timezone"].as_str().map(String::from),
        source: "ipapi.co".into(),
        approximate: true,
        detected_at: chrono::Utc::now().to_rfc3339(),
    })
}
pub fn needs_location(ask: &str) -> bool {
    let s = ask.to_lowercase();
    [
        "weather",
        "forecast",
        "near me",
        "nearby",
        "around here",
        "my area",
        "local news",
    ]
    .iter()
    .any(|p| s.contains(p))
}
pub fn explicit_place(ask: &str) -> bool {
    let lower = ask.to_lowercase();
    lower.split_once(" in ").is_some_and(|(_, s)| {
        ![
            "my area",
            "my city",
            "my location",
            "my town",
            "this area",
            "here",
            "celsius",
            "fahrenheit",
            "kelvin",
        ]
        .iter()
        .any(|p| s.starts_with(p))
    }) || lower.split_once(" for ").is_some_and(|(_, s)| {
        ![
            "today", "tomorrow", "this", "next", "the week", "my area", "me",
        ]
        .iter()
        .any(|p| s.starts_with(p))
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[tokio::test]
    async fn overrides_disabled_and_invalid_responses() {
        let service = LocationService::default();
        let cfg = LocationCfg {
            city: "Test City".into(),
            ..Default::default()
        };
        assert_eq!(service.resolve(&cfg, false).await.unwrap().source, "saved");
        assert!(service
            .resolve(
                &LocationCfg {
                    enabled: false,
                    ..cfg.clone()
                },
                false
            )
            .await
            .is_err());
        assert!(LocationCfg {
            latitude: Some(0.),
            ..cfg.clone()
        }
        .validate()
        .is_err());
        assert!(LocationCfg {
            latitude: Some(91.),
            longitude: Some(0.),
            ..cfg
        }
        .validate()
        .is_err());
        assert!(decode_ip(&json!({"error":true})).is_err());
        assert!(decode_ip(&json!({"city":"Test","latitude":999,"longitude":0})).is_err());
        assert!(
            decode_ip(&json!({"city":"Test","latitude":0,"longitude":0}))
                .unwrap()
                .approximate
        );
        let fallback = decode_provider(&json!({"success":true,"city":"Test","country":"Country","latitude":0,"longitude":0,"timezone":{"id":"UTC"}}), "ipwho.is").unwrap();
        assert_eq!(fallback.source, "ipwho.is");
        assert_eq!(fallback.timezone.as_deref(), Some("UTC"));
        assert!(decode_provider(&json!({"success":false}), "ipwho.is").is_err());
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("config.toml");
        std::fs::write(&path,"[location]\ncity = \"Old city\"\nlatitude = 10.0\nlongitude = 20.0\nvendor = \"keep\" # keep comment\n").unwrap();
        let mut config = crate::Config::load_from(&path).unwrap();
        config.location.city = "New city".into();
        config.location.latitude = None;
        config.location.longitude = None;
        config.save_to(&path).unwrap();
        let reloaded = crate::Config::load_from(&path).unwrap();
        assert!(reloaded.location.latitude.is_none());
        assert!(std::fs::read_to_string(&path)
            .unwrap()
            .contains("# keep comment"));
        assert!(needs_location("cafes near me"));
        assert!(!explicit_place("forecast for tomorrow"));
        assert!(explicit_place("weather in Chicago"));
    }
}
