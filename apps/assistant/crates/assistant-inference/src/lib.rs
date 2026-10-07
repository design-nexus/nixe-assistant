//! Resident llama.cpp CUDA backend.

use anyhow::{Context, Result};
use assistant_core::config::Config;
use encoding_rs::UTF_8;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::send_logs_to_tracing;
use llama_cpp_2::LogOptions;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use tokio::sync::{mpsc, oneshot};

pub struct LlamaCppBackend {
    jobs: mpsc::Sender<Job>,
    loaded: Arc<AtomicBool>,
    model_path: PathBuf,
}

enum Job {
    Generate {
        system: String,
        history: Vec<(String, String)>,
        user: String,
        n_predict: i32,
        temperature: f32,
        tokens: mpsc::Sender<String>,
        cancel: Arc<AtomicBool>,
        done: oneshot::Sender<Result<()>>,
    },
}

impl LlamaCppBackend {
    pub fn spawn(cfg: &Config) -> Result<Self> {
        let model_path = cfg.resolve_model_path();
        if !model_path.is_file() {
            anyhow::bail!(
                "GGUF not found at {}. Set [llm].model to a local .gguf path.",
                model_path.display()
            );
        }
        let worker_cfg = WorkerConfig {
            n_gpu_layers: if cfg.llm.n_gpu_layers < 0 {
                1000
            } else {
                cfg.llm.n_gpu_layers as u32
            },
            ctx_size: cfg.llm.ctx,
            flash_attn: cfg.llm.flash_attn,
            prefix_cache: cfg.llm.prefix_cache,
            n_batch: cfg.llm.n_batch,
            n_ubatch: cfg.llm.n_ubatch,
        };
        let loaded = Arc::new(AtomicBool::new(false));
        let loaded_w = loaded.clone();
        let (jobs_tx, mut jobs_rx) = mpsc::channel::<Job>(4);
        let path = model_path.clone();

        thread::Builder::new()
            .name("nyx-llama".into())
            .spawn(move || {
                if let Err(err) = worker(path, worker_cfg, loaded_w, &mut jobs_rx) {
                    tracing::error!(%err, "llama worker died");
                }
            })
            .context("spawn llama worker")?;

        Ok(Self {
            jobs: jobs_tx,
            loaded,
            model_path,
        })
    }

    pub fn model_path(&self) -> &std::path::Path {
        &self.model_path
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded.load(Ordering::Relaxed)
    }

    pub async fn generate(
        &self,
        system: &str,
        history: &[(String, String)],
        user: &str,
        n_predict: i32,
        temperature: f32,
        cancel: Arc<AtomicBool>,
    ) -> Result<mpsc::Receiver<String>> {
        let (tokens, rx) = mpsc::channel(64);
        let (done_tx, done_rx) = oneshot::channel();
        self.jobs
            .send(Job::Generate {
                system: system.to_string(),
                history: history.to_vec(),
                user: user.to_string(),
                n_predict,
                temperature,
                tokens,
                cancel,
                done: done_tx,
            })
            .await
            .map_err(|_| anyhow::anyhow!("llama worker gone"))?;
        tokio::spawn(async move {
            if let Ok(Err(err)) = done_rx.await {
                tracing::warn!(%err, "generation failed");
            }
        });
        Ok(rx)
    }
}

#[derive(Clone, Copy)]
struct WorkerConfig {
    n_gpu_layers: u32,
    ctx_size: u32,
    flash_attn: bool,
    prefix_cache: bool,
    n_batch: u32,
    n_ubatch: u32,
}

fn worker(
    path: PathBuf,
    w_cfg: WorkerConfig,
    loaded: Arc<AtomicBool>,
    jobs: &mut mpsc::Receiver<Job>,
) -> Result<()> {
    send_logs_to_tracing(LogOptions::default().with_logs_enabled(false));
    let backend = LlamaBackend::init().context("llama backend")?;
    let model_params = LlamaModelParams::default().with_n_gpu_layers(w_cfg.n_gpu_layers);
    tracing::info!(
        path = %path.display(),
        n_gpu_layers = w_cfg.n_gpu_layers,
        flash_attn = w_cfg.flash_attn,
        prefix_cache = w_cfg.prefix_cache,
        "loading GGUF"
    );
    let t0 = std::time::Instant::now();
    let model = LlamaModel::load_from_file(&backend, &path, &model_params)
        .with_context(|| format!("load {}", path.display()))?;

    let flash_policy = if w_cfg.flash_attn {
        llama_cpp_sys_2::LLAMA_FLASH_ATTN_TYPE_ENABLED
    } else {
        llama_cpp_sys_2::LLAMA_FLASH_ATTN_TYPE_DISABLED
    };

    let ctx_params = LlamaContextParams::default()
        .with_n_ctx(Some(NonZeroU32::new(w_cfg.ctx_size).unwrap_or(NonZeroU32::new(4096).unwrap())))
        .with_flash_attention_policy(flash_policy)
        .with_n_batch(w_cfg.n_batch)
        .with_n_ubatch(w_cfg.n_ubatch);

    let mut ctx = model
        .new_context(&backend, ctx_params)
        .context("llama context")?;
    tracing::info!(ms = t0.elapsed().as_millis(), "GGUF loaded");
    loaded.store(true, Ordering::Relaxed);

    let mut cached_tokens: Vec<llama_cpp_2::token::LlamaToken> = Vec::new();

    while let Some(job) = jobs.blocking_recv() {
        match job {
            Job::Generate {
                system,
                history,
                user,
                n_predict,
                temperature,
                tokens,
                cancel,
                done,
            } => {
                let r = generate_one(
                    &model,
                    &mut ctx,
                    &mut cached_tokens,
                    w_cfg.prefix_cache,
                    w_cfg.ctx_size,
                    &system,
                    &history,
                    &user,
                    n_predict,
                    temperature,
                    tokens,
                    cancel,
                );
                let _ = done.send(r);
            }
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn generate_one(
    model: &LlamaModel,
    ctx: &mut llama_cpp_2::context::LlamaContext<'_>,
    cached_tokens: &mut Vec<llama_cpp_2::token::LlamaToken>,
    prefix_cache_enabled: bool,
    ctx_size: u32,
    system: &str,
    history: &[(String, String)],
    user: &str,
    n_predict: i32,
    temperature: f32,
    tokens_tx: mpsc::Sender<String>,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let prompt = chat_prompt(model, system, history, user)?;
    let tokens_list = model.vocab().tokenize(prompt.as_bytes(), true, true);
    let n_prompt = tokens_list.len();

    // Prevent KV-cache overflow
    if cached_tokens.len() + 64 >= ctx_size as usize {
        ctx.clear_kv_cache();
        cached_tokens.clear();
    }

    let common_len = if prefix_cache_enabled {
        let matching = tokens_list
            .iter()
            .zip(cached_tokens.iter())
            .take_while(|(a, b)| a == b)
            .count();
        // If matching equals the entire prompt, back off by 1 so the last token is re-decoded to yield logits.
        if matching == n_prompt && matching > 0 {
            matching - 1
        } else {
            matching
        }
    } else {
        0
    };

    let mut batch = if common_len > 0 {
        tracing::debug!(common_len, total = n_prompt, "reusing cached KV prefix");
        let _ = ctx.clear_kv_cache_seq(None, Some(common_len as u32), None);
        cached_tokens.truncate(common_len);
        let suffix = &tokens_list[common_len..];
        let n_suffix = suffix.len();
        let mut b = LlamaBatch::new(n_suffix.max(512), 1);
        for (offset, &token) in suffix.iter().enumerate() {
            let pos = (common_len + offset) as i32;
            let is_last = offset == n_suffix - 1;
            b.add(token, pos, &[0], is_last)?;
        }
        ctx.decode(&mut b).context("decode prompt suffix")?;
        cached_tokens.extend_from_slice(suffix);
        b
    } else {
        ctx.clear_kv_cache();
        cached_tokens.clear();
        let mut b = LlamaBatch::new(n_prompt.max(512), 1);
        for (i, &token) in tokens_list.iter().enumerate() {
            let pos = i as i32;
            let is_last = i == n_prompt - 1;
            b.add(token, pos, &[0], is_last)?;
        }
        ctx.decode(&mut b).context("decode prompt")?;
        cached_tokens.extend_from_slice(&tokens_list);
        b
    };

    let mut sampler = if temperature <= 0.01 {
        LlamaSampler::greedy()
    } else {
        LlamaSampler::chain(
            [LlamaSampler::temp(temperature), LlamaSampler::dist(1234)],
            false,
        )
    };
    let mut decoder = UTF_8.new_decoder();
    let mut n_cur = n_prompt as i32;
    let n_max = n_cur + n_predict.max(1);

    while n_cur < n_max {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let token = sampler.sample(ctx, batch.n_tokens() - 1);
        sampler.accept(token);
        if model.vocab().is_eog(token) {
            break;
        }
        let piece = model.vocab().token_to_piece(token, true, None);
        let mut out = String::with_capacity(decoder.max_utf8_buffer_length(piece.len()).unwrap_or(piece.len()));
        let _ = decoder.decode_to_string(&piece, &mut out, false);
        if !out.is_empty() {
            let _ = tokens_tx.blocking_send(out);
        }
        batch.clear();
        batch.add(token, n_cur, &[0], true)?;
        ctx.decode(&mut batch).context("decode token")?;
        cached_tokens.push(token);
        n_cur += 1;
    }
    Ok(())
}

fn chat_prompt(
    model: &LlamaModel,
    system: &str,
    history: &[(String, String)],
    user: &str,
) -> Result<String> {
    if let Ok(tmpl) = model.chat_template(None) {
        if tmpl
            .to_str()
            .map(|s| s.contains("<|turn>"))
            .unwrap_or(false)
        {
            tracing::info!(turns = history.len(), "using Gemma 4 <|turn|> chat format");
            return Ok(gemma4_prompt(system, history, user));
        }
        let mut msgs = Vec::with_capacity(history.len() * 2 + 2);
        msgs.push(
            LlamaChatMessage::new("system".into(), system.into())
                .or_else(|_| LlamaChatMessage::new("user".into(), system.into()))
                .map_err(|e| anyhow::anyhow!("{e}"))?,
        );
        for (u, a) in history {
            msgs.push(LlamaChatMessage::new("user".into(), u.clone()).map_err(|e| anyhow::anyhow!("{e}"))?);
            msgs.push(
                LlamaChatMessage::new("assistant".into(), a.clone())
                    .or_else(|_| LlamaChatMessage::new("model".into(), a.clone()))
                    .map_err(|e| anyhow::anyhow!("{e}"))?,
            );
        }
        msgs.push(LlamaChatMessage::new("user".into(), user.into()).map_err(|e| anyhow::anyhow!("{e}"))?);
        if let Ok(formatted) = model.apply_chat_template(&tmpl, &msgs, true) {
            return Ok(formatted);
        }
        tracing::warn!("llama.cpp could not apply the GGUF chat template");
    }
    Ok(gemma4_prompt(system, history, user))
}

/// llama.cpp's `llama_chat_apply_template` does not evaluate Gemma 4's Jinja
/// (macros, tools). The User:/Assistant: fallback makes the model continue a
/// fake dialogue — it sounds like it is reading a list of prompts and replies.
fn gemma4_prompt(system: &str, history: &[(String, String)], user: &str) -> String {
    let mut out = String::from("<|turn>system\n");
    out.push_str(system);
    out.push_str("<turn|>\n");
    for (u, a) in history {
        out.push_str("<|turn>user\n");
        out.push_str(u);
        out.push_str("<turn|>\n<|turn>model\n");
        out.push_str(a);
        out.push_str("<turn|>\n");
    }
    out.push_str("<|turn>user\n");
    out.push_str(user);
    out.push_str("<turn|>\n<|turn>model\n");
    out
}

#[cfg(test)]
mod tests {
    use super::gemma4_prompt;

    #[test]
    fn gemma4_prompt_opens_model_turn() {
        let p = gemma4_prompt("sys", &[], "hello");
        assert!(p.starts_with("<|turn>system\n"));
        assert!(p.contains("<|turn>user\nhello<turn|>"));
        assert!(p.ends_with("<|turn>model\n"));
        assert!(!p.contains("User:"));
        assert!(!p.contains("Assistant:"));
    }

    #[test]
    fn gemma4_prompt_includes_prior_turns() {
        let hist = vec![("hi".into(), "hello".into())];
        let p = gemma4_prompt("sys", &hist, "again");
        assert!(p.contains("<|turn>user\nhi<turn|>"));
        assert!(p.contains("<|turn>model\nhello<turn|>"));
        assert!(p.contains("<|turn>user\nagain<turn|>"));
        assert!(p.ends_with("<|turn>model\n"));
    }
}
