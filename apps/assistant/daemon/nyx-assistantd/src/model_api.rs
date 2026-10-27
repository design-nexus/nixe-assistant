//! Model API v1: correlated, tool-free requests over Assistant's private local socket.
use super::App;
use anyhow::{bail, Context, Result};
use assistant_core::config::Config;
use serde_json::{json, Value};
use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::Duration,
};
pub(super) fn capabilities(cfg: &Config) -> Value {
    json!({"api_version":1,"generation":true,"local":true,"embedding":cfg.embeddings.enabled&&!cfg.embeddings.endpoint.is_empty()&&!cfg.embeddings.model.is_empty(),"embedding_model":cfg.embeddings.model,"embedding_profile":format!("{}\n{}",cfg.embeddings.model,cfg.embeddings.endpoint),"embedding_endpoint":cfg.embeddings.endpoint})
}
pub(super) async fn generate(
    app: &Arc<App>,
    system: String,
    text: String,
    max_tokens: Option<i32>,
    temperature: Option<f32>,
) -> Result<Value> {
    if system.len() > 32768 || text.len() > 128 * 1024 || text.trim().is_empty() {
        bail!("Model request exceeds limits or is empty");
    }
    let cfg = app.cfg();
    let limit = max_tokens.unwrap_or(cfg.llm.n_predict).clamp(1, 4096);
    let temperature = temperature.unwrap_or(0.3);
    if !temperature.is_finite() || !(0.0..=2.0).contains(&temperature) {
        bail!("Invalid temperature");
    }
    if app.turn_busy.swap(true, Ordering::SeqCst) {
        bail!("Assistant is busy; retry when the current turn finishes");
    }
    let generation = app.turn_gen.fetch_add(1, Ordering::SeqCst) + 1;
    let interrupted = app.reset_cancel().await;
    let cancel = Arc::new(AtomicBool::new(false));
    let result = tokio::time::timeout(Duration::from_secs(120), async {
        // Calling the resident model directly deliberately bypasses agents, tools, speech,
        // conversation history, and network retrieval. Note contents remain untrusted data.
        let mut tokens = app.llm.generate(&system, &[], &text, limit, temperature, cancel.clone()).await?;
        let mut output = String::new();
        loop {
            let token =
                tokio::select! {_ = interrupted.cancelled()=>bail!("Model generation cancelled"),token=tokens.recv()=>token};
            let Some(token) = token else {
                break;
            };
            output.push_str(&token?);
            if output.len() > 1024 * 1024 {
                bail!("Model response exceeds limit");
            }
        }
        Ok::<_, anyhow::Error>(json!({"api_version":1,"text":output,"local":true}))
    })
    .await;
    cancel.store(true, Ordering::Relaxed);
    app.finish_turn(generation);
    result.context("Model generation timed out")?
}
pub(super) async fn embed(cfg: &Config, input: Vec<String>) -> Result<Value> {
    let settings = &cfg.embeddings;
    if !settings.enabled || settings.endpoint.is_empty() || settings.model.is_empty() {
        bail!("Local embeddings are not configured in Assistant");
    }
    if input.is_empty() || input.len() > 32 || input.iter().map(String::len).sum::<usize>() > 128 * 1024 {
        bail!("Embedding batch exceeds limits");
    }
    let url = reqwest::Url::parse(&settings.endpoint).context("Invalid embeddings endpoint")?;
    if !local_url(&url) {
        bail!("Embeddings endpoints must use an explicit loopback address; cloud uploads are not supported");
    }
    let client = reqwest::Client::builder()
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .timeout(Duration::from_secs(90))
        .build()?;
    let count = input.len();
    let mut response = client
        .post(url)
        .json(&json!({"model":settings.model,"input":input,"encoding_format":"float"}))
        .send()
        .await?
        .error_for_status()?;
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await? {
        if bytes.len() + chunk.len() > 8 * 1024 * 1024 {
            bail!("Embedding response exceeds limit");
        }
        bytes.extend_from_slice(&chunk);
    }
    let value: Value = serde_json::from_slice(&bytes)?;
    let data = value["data"].as_array().context("Embedding response needs a data array")?;
    if data.len() != count {
        bail!("Wrong embedding batch size");
    }
    let mut ordered = vec![None; count];
    let mut dimensions = None;
    for row in data {
        let index = row["index"].as_u64().context("Missing embedding index")? as usize;
        if index >= count || ordered[index].is_some() {
            bail!("Invalid embedding index");
        }
        let vector = row["embedding"]
            .as_array()
            .context("Missing embedding vector")?
            .iter()
            .map(|v| {
                v.as_f64()
                    .filter(|v| v.is_finite())
                    .map(|n| n as f32)
                    .filter(|n| n.is_finite())
                    .context("Non-finite embedding value")
            })
            .collect::<Result<Vec<_>>>()?;
        if vector.is_empty() || vector.len() > 16384 || dimensions.is_some_and(|d| d != vector.len()) {
            bail!("Invalid embedding dimensions");
        }
        dimensions = Some(vector.len());
        ordered[index] = Some(vector);
    }
    Ok(
        json!({"api_version":1,"model":settings.model,"profile":format!("{}\n{}",settings.model,settings.endpoint),"vectors":ordered.into_iter().collect::<Option<Vec<_>>>().context("Missing embedding")?}),
    )
}
fn local_url(url: &reqwest::Url) -> bool {
    if url.scheme() != "http" && url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
    {
        return false;
    }
    url.host_str().is_some_and(|host| host.trim_matches(['[', ']']).parse::<std::net::IpAddr>().is_ok_and(|ip| ip.is_loopback()))
}

pub(super) fn configure(app: &Arc<App>, enabled: bool, endpoint: String, model: String) -> Result<Value> {
    if endpoint.len() > 4096 || model.len() > 512 {
        bail!("Embedding configuration exceeds limits");
    }
    if enabled && (model.trim().is_empty() || !reqwest::Url::parse(&endpoint).is_ok_and(|u| local_url(&u))) {
        bail!("Choose a model and an explicit loopback embeddings URL");
    }
    let mut cfg = app.cfg();
    cfg.embeddings = assistant_core::config::EmbeddingsCfg { enabled, endpoint, model };
    cfg.save()?;
    *app.cfg.lock().unwrap_or_else(|e| e.into_inner()) = cfg.clone();
    Ok(capabilities(&cfg))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn model_api_is_local_and_opt_in() {
        let cfg = Config::default();
        let caps = capabilities(&cfg);
        assert_eq!(caps["api_version"], 1);
        assert_eq!(caps["embedding"], false);
        assert!(local_url(&reqwest::Url::parse("http://127.0.0.1:1234/v1/embeddings").unwrap()));
        assert!(local_url(&reqwest::Url::parse("http://[::1]:1234/v1/embeddings").unwrap()));
        for url in [
            "https://example.com/v1/embeddings",
            "http://localhost.example/embeddings",
            "file:///etc/passwd",
            "http://user:pass@127.0.0.1/",
        ] {
            assert!(!local_url(&reqwest::Url::parse(url).unwrap()));
        }
    }
}
