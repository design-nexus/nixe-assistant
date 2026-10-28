//! Bounded, on-demand definitions from the complete live registry.
use serde_json::{json, Value};
pub const SCHEMA_BUDGET: usize = 10_000;
pub const ALWAYS: &[&str] = &["tools.discover", "canvas.show", "location.current", "location.search"];
pub fn bounded(catalog: &[Value], keep: &[&str]) -> Vec<Value> {
    let mut out = Vec::new();
    let mut bytes = 2;
    for tool in catalog
        .iter()
        .filter(|t| keep.contains(&t["name"].as_str().unwrap_or("")))
        .chain(catalog.iter().filter(|t| !keep.contains(&t["name"].as_str().unwrap_or(""))))
    {
        let size = tool.to_string().len() + 1;
        if bytes + size > SCHEMA_BUDGET || out.len() >= 16 {
            continue;
        }
        if out.iter().any(|t: &Value| t["name"] == tool["name"]) {
            continue;
        }
        bytes += size;
        out.push(tool.clone());
    }
    out
}
pub fn groups(catalog: &[Value]) -> Vec<String> {
    catalog
        .iter()
        .filter_map(|t| t["name"].as_str())
        .map(|n| n.split('.').next().unwrap_or(n).to_string())
        .collect::<std::collections::BTreeSet<_>>()
        .into_iter()
        .collect()
}
pub fn discover(catalog: &[Value], query: &str, limit: usize) -> Value {
    let lower = query.to_lowercase();
    let words: Vec<_> = lower.split(|c: char| !c.is_alphanumeric()).filter(|w| !w.is_empty() && !["a","an","the","it","me","my","please","make","show","can","you","to","in","on","with","is","that","this","do","now"].contains(w)).collect();
    let mut ranked: Vec<_> = catalog
        .iter()
        .filter_map(|tool| {
            let name = tool["name"].as_str().unwrap_or("").to_lowercase();
            let desc = tool["description"].as_str().unwrap_or("").to_lowercase();
            let score = if name == lower && !lower.is_empty() {
                10_000
            } else {
                words.iter().map(|w| usize::from(name.contains(w)) * 8 + usize::from(desc.contains(w))).sum()
            };
            (words.is_empty() || score > 0).then_some((score, tool))
        })
        .collect();
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then(a.1["name"].as_str().cmp(&b.1["name"].as_str())));
    let matching = ranked.len();
    let mut tools = Vec::new();
    let mut bytes = 0;
    for (_, tool) in ranked.into_iter().take(limit.clamp(1, 8)) {
        let size = tool.to_string().len();
        if bytes + size > 6000 {
            continue;
        }
        bytes += size;
        tools.push(tool.clone());
    }
    json!({"query":query,"groups":groups(catalog),"total_registered":catalog.len(),"matching":matching,"more":matching>tools.len(),"tools":tools,"note":"These are definitions, not permission grants. Search a specific tool or group for other definitions."})
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn complete_registry_discovery_with_bounded_schemas() {
        let mut catalog = Vec::new();
        for i in 0..300 {
            catalog.push(json!({"name":format!("group{i}.run"),"description":"Registered action","inputSchema":{"type":"object"},"permission":"confirm"}));
        }
        catalog.push(json!({"name":"canvas.show","inputSchema":{"type":"object"},"permission":"allow"}));
        for tool in &catalog {
            let found = discover(&catalog, tool["name"].as_str().unwrap(), 1);
            assert_eq!(found["tools"][0]["name"], tool["name"]);
            assert_eq!(found["tools"][0]["permission"], tool["permission"]);
        }
        let selected = bounded(&catalog, ALWAYS);
        assert!(selected.iter().any(|t| t["name"] == "canvas.show"));
        assert!(serde_json::to_string(&selected).unwrap().len() <= SCHEMA_BUDGET);
        assert!(selected.len() <= 16);
    }
}
