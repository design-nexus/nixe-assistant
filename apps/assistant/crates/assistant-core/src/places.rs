//! Bounded named-place lookup. Candidates are evidence, never invented coordinates.
use anyhow::{Context, Result};
use serde_json::{json, Value};
use std::time::Duration;
const STATES: &str = "AL:Alabama|AK:Alaska|AZ:Arizona|AR:Arkansas|CA:California|CO:Colorado|CT:Connecticut|DE:Delaware|FL:Florida|GA:Georgia|HI:Hawaii|ID:Idaho|IL:Illinois|IN:Indiana|IA:Iowa|KS:Kansas|KY:Kentucky|LA:Louisiana|ME:Maine|MD:Maryland|MA:Massachusetts|MI:Michigan|MN:Minnesota|MS:Mississippi|MO:Missouri|MT:Montana|NE:Nebraska|NV:Nevada|NH:New Hampshire|NJ:New Jersey|NM:New Mexico|NY:New York|NC:North Carolina|ND:North Dakota|OH:Ohio|OK:Oklahoma|OR:Oregon|PA:Pennsylvania|RI:Rhode Island|SC:South Carolina|SD:South Dakota|TN:Tennessee|TX:Texas|UT:Utah|VT:Vermont|VA:Virginia|WA:Washington|WV:West Virginia|WI:Wisconsin|WY:Wyoming|DC:District of Columbia";
pub fn normalize(query: &str) -> String {
    // Spoken region names often arrive without a comma.
    if !query.contains(',') {
        let lower = query.to_lowercase();
        let suffix = STATES
            .split('|')
            .filter_map(|s| s.split_once(':'))
            .flat_map(|(abbr, name)| [(abbr, name), (name, name)])
            .filter(|(suffix, _)| lower.ends_with(&format!(" {}", suffix.to_lowercase())))
            .max_by_key(|(suffix, _)| suffix.len());
        if let Some((suffix, name)) = suffix {
            let city = query[..query.len() - suffix.len()].trim();
            if !city.is_empty() {
                return format!("{city}, {name}");
            }
        }
    }
    let mut parts = query.split(',').map(str::trim);
    let city = parts.next().unwrap_or("");
    let region = parts.next().unwrap_or("");
    let state = STATES
        .split('|')
        .filter_map(|s| s.split_once(':'))
        .find(|(abbr, name)| region.eq_ignore_ascii_case(abbr) || region.eq_ignore_ascii_case(name));
    if let Some((_, name)) = state {
        format!("{city}, {name}")
    } else {
        query.trim().into()
    }
}
fn distance(a: &str, b: &str) -> usize {
    let mut row: Vec<usize> = (0..=b.chars().count()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, y) in b.chars().enumerate() {
            let old = row[j + 1];
            row[j + 1] = (row[j] + 1).min(old + 1).min(diagonal + usize::from(x != y));
            diagonal = old;
        }
    }
    *row.last().unwrap()
}
pub async fn search(query: &str, alternatives: &[String]) -> Result<Value> {
    anyhow::ensure!(!query.trim().is_empty() && query.len() <= 256 && alternatives.len() <= 3, "invalid place query");
    let client =
        reqwest::Client::builder().timeout(Duration::from_secs(5)).redirect(reqwest::redirect::Policy::none()).build()?;
    let normalized = normalize(query);
    let name = normalized.split(',').next().unwrap_or("").to_lowercase();
    let region = normalized.split(',').nth(1).map(str::trim).unwrap_or("");
    let mut queries = vec![normalized.clone()];
    // Alternatives may change a misheard name, never the explicitly supplied region.
    for alt in alternatives.iter().filter(|a| !a.is_empty() && a.len() <= 128) {
        let city = alt.split(',').next().unwrap_or("").trim();
        let q = if region.is_empty() { normalize(alt) } else { format!("{city}, {region}") };
        if !queries.contains(&q) {
            queries.push(q);
        }
    }
    let mut candidates = Vec::new();
    for (index, q) in queries.iter().enumerate() {
        let mut response = client
            .get("https://geocoding-api.open-meteo.com/v1/search")
            .query(&[("name", q.as_str()), ("count", "20"), ("language", "en"), ("format", "json")])
            .send()
            .await?
            .error_for_status()?;
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            anyhow::ensure!(bytes.len() + chunk.len() <= 65536, "place response too large");
            bytes.extend_from_slice(&chunk);
        }
        let value: Value = serde_json::from_slice(&bytes)?;
        anyhow::ensure!(value["error"] != true, "place provider rejected the query");
        for place in value["results"].as_array().into_iter().flatten() {
            let Some(label) = place["name"].as_str() else { continue };
            let lat = place["latitude"].as_f64().context("missing latitude")?;
            let lon = place["longitude"].as_f64().context("missing longitude")?;
            if !lat.is_finite() || !lon.is_finite() || !(-85.0..=85.0).contains(&lat) || !(-180.0..=180.0).contains(&lon) {
                continue;
            }
            let state = place["admin1"].as_str().unwrap_or("");
            let country = place["country"].as_str().unwrap_or("");
            let code = place["country_code"].as_str().unwrap_or("");
            // Enforce region even if a provider unexpectedly returns a broader match.
            if !region.is_empty() && ![state, country, code].iter().any(|s| s.eq_ignore_ascii_case(region)) {
                continue;
            }
            let exact = index == 0 && label.eq_ignore_ascii_case(&name);
            let candidate = json!({"name":label,"region":state,"country":country,"latitude":lat,"longitude":lon,"exact":exact,"distance":distance(&name,&label.to_lowercase()),"source":"Open-Meteo / GeoNames"});
            if !candidates
                .iter()
                .any(|p: &Value| p["latitude"] == candidate["latitude"] && p["longitude"] == candidate["longitude"])
            {
                candidates.push(candidate);
            }
        }
        if candidates.iter().any(|p| p["exact"] == true) {
            break;
        }
    }
    candidates.sort_by_key(|p| (p["exact"] != true, p["distance"].as_u64().unwrap_or(999)));
    candidates.truncate(8);
    Ok(
        json!({"query":query,"normalized_query":normalized,"status":if candidates.is_empty(){"no_match"}else{"matches"},"candidates":candidates,"note":"A failed lookup does not prove a place does not exist. Ask which candidate the user means when ambiguous. Alternatives require confirmation. Never invent coordinates."}),
    )
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn regions_and_similar_names() {
        assert_eq!(normalize("Calera, AL"), "Calera, Alabama");
        assert_eq!(normalize("Calera Alabama"), "Calera, Alabama");
        assert_eq!(normalize("Calera al"), "Calera, Alabama");
        assert_eq!(normalize("Charleston West Virginia"), "Charleston, West Virginia");
        assert_eq!(normalize("Paris, France"), "Paris, France");
        assert_eq!(distance("calera", "camera"), 1);
    }
}
