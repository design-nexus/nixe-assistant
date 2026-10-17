//! Resident llama.cpp CUDA backend.

use anyhow::{Context, Result};
use assistant_core::config::Config;
use encoding_rs::UTF_8;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaModel};
use llama_cpp_2::mtmd::{mtmd_default_marker, MtmdBitmap, MtmdContext, MtmdContextParams, MtmdInputChunkType, MtmdInputText};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::send_logs_to_tracing;
use llama_cpp_2::LogOptions;
use std::num::NonZeroU32;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use tokio::sync::{mpsc, oneshot};

pub struct LlamaCppBackend {
    jobs: mpsc::Sender<Job>,
    loaded: Arc<AtomicBool>,
    model_path: PathBuf,
    battery: Arc<AtomicBool>,
    mmproj: Option<PathBuf>,
    vision_idle_secs: u64,
    vision_loaded: Arc<AtomicBool>,
}

enum Job {
    /// Answer about an image, loading the projector if needed.
    Look {
        image: Vec<u8>,
        prompt: String,
        n_predict: i32,
        done: oneshot::Sender<Result<String>>,
    },
    /// Unload the projector if no look happened since `since` (seconds since the epoch).
    DropVision {
        since: u64,
    },
    Unload {
        done: oneshot::Sender<()>,
    },
    Power {
        battery: bool,
        done: oneshot::Sender<()>,
    },
    Generate {
        system: String,
        history: Vec<(String, String)>,
        user: String,
        n_predict: i32,
        temperature: f32,
        tokens: mpsc::Sender<Result<String>>,
        grammar: Option<String>,
        cancel: Arc<AtomicBool>,
        done: oneshot::Sender<Result<()>>,
    },
}

impl LlamaCppBackend {
    pub fn spawn(cfg: &Config) -> Result<Self> {
        Self::spawn_with_battery(cfg, false)
    }
    pub fn spawn_with_battery(cfg: &Config, battery_mode: bool) -> Result<Self> {
        let model_path = cfg.resolve_model_path();
        if !model_path.is_file() {
            anyhow::bail!(
                "GGUF not found at {}. Set [llm].model to a local .gguf path.",
                model_path.display()
            );
        }
        let configured_gpu_layers = if cfg.llm.n_gpu_layers < 0 {
            1000
        } else {
            cfg.llm.n_gpu_layers as u32
        };
        let worker_cfg = WorkerConfig {
            n_gpu_layers: if battery_mode {
                0
            } else {
                configured_gpu_layers
            },
            ctx_size: cfg.llm.ctx,
            flash_attn: cfg.llm.flash_attn,
            prefix_cache: cfg.llm.prefix_cache,
            n_batch: cfg.llm.n_batch,
            n_ubatch: cfg.llm.n_ubatch,
        };
        let loaded = Arc::new(AtomicBool::new(false));
        let loaded_w = loaded.clone();
        let battery = Arc::new(AtomicBool::new(battery_mode));
        let battery_w = battery.clone();
        let (jobs_tx, mut jobs_rx) = mpsc::channel::<Job>(4);
        let path = model_path.clone();
        let mmproj = cfg.vision.resolve(&model_path);
        let mmproj_w = mmproj.clone();
        let last_look = Arc::new(AtomicU64::new(0));
        let last_look_w = last_look.clone();
        let vision_loaded = Arc::new(AtomicBool::new(false));
        let vision_loaded_w = vision_loaded.clone();

        thread::Builder::new()
            .name("assistant-llama".into())
            .spawn(move || {
                if let Err(err) = worker(
                    path,
                    worker_cfg,
                    configured_gpu_layers,
                    loaded_w,
                    battery_w,
                    mmproj_w,
                    last_look_w,
                    vision_loaded_w,
                    &mut jobs_rx,
                ) {
                    tracing::error!(%err, "llama worker died");
                }
            })
            .context("spawn llama worker")?;

        Ok(Self {
            jobs: jobs_tx,
            loaded,
            model_path,
            battery,
            mmproj,
            vision_idle_secs: cfg.vision.idle_secs.max(10),
            vision_loaded,
        })
    }

    /// Whether the image projector is in memory now.
    pub fn vision_loaded(&self) -> bool {
        self.vision_loaded.load(Ordering::Relaxed)
    }

    /// The image projector looks will use, if one was found.
    pub fn vision_model(&self) -> Option<&std::path::Path> {
        self.mmproj.as_deref()
    }

    /// Answer `prompt` about a PNG. The projector loads on the first look and unloads
    /// after `[vision] idle_secs` without one.
    pub async fn look(&self, image: Vec<u8>, prompt: &str, n_predict: i32) -> Result<String> {
        anyhow::ensure!(self.mmproj.is_some(), "no image projector (mmproj) next to the model; set [vision] mmproj");
        let (done, answer) = oneshot::channel();
        self.jobs
            .send(Job::Look { image, prompt: prompt.into(), n_predict, done })
            .await
            .context("llama worker gone")?;
        let text = answer.await.context("look failed")??;
        let since = epoch_secs();
        let jobs = self.jobs.clone();
        let idle = self.vision_idle_secs;
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(idle)).await;
            let _ = jobs.send(Job::DropVision { since }).await;
        });
        Ok(text)
    }

    pub fn model_path(&self) -> &std::path::Path {
        &self.model_path
    }

    pub fn is_loaded(&self) -> bool {
        self.loaded.load(Ordering::Relaxed)
    }

    pub fn is_battery(&self) -> bool {
        self.battery.load(Ordering::Relaxed)
    }
    pub async fn unload(&self) -> Result<()> {
        let (done, ack) = oneshot::channel();
        self.jobs
            .send(Job::Unload { done })
            .await
            .context("llama worker gone")?;
        ack.await.context("model unload failed")?;
        Ok(())
    }
    pub async fn set_battery(&self, battery: bool) -> Result<()> {
        let (done, ack) = oneshot::channel();
        self.jobs
            .send(Job::Power { battery, done })
            .await
            .context("llama worker gone")?;
        ack.await.context("power change failed")?;
        Ok(())
    }

    pub async fn generate(
        &self,
        system: &str,
        history: &[(String, String)],
        user: &str,
        n_predict: i32,
        temperature: f32,
        cancel: Arc<AtomicBool>,
    ) -> Result<mpsc::Receiver<Result<String>>> {
        self.generate_with_grammar(system, history, user, n_predict, temperature, cancel, None)
            .await
    }

    #[allow(clippy::too_many_arguments)]
    pub async fn generate_with_grammar(
        &self,
        system: &str,
        history: &[(String, String)],
        user: &str,
        n_predict: i32,
        temperature: f32,
        cancel: Arc<AtomicBool>,
        grammar: Option<String>,
    ) -> Result<mpsc::Receiver<Result<String>>> {
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
                grammar,
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

fn epoch_secs() -> u64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

#[allow(clippy::too_many_arguments)]
fn worker(
    path: PathBuf,
    mut w_cfg: WorkerConfig,
    configured_gpu_layers: u32,
    loaded: Arc<AtomicBool>,
    battery: Arc<AtomicBool>,
    mmproj: Option<PathBuf>,
    last_look: Arc<AtomicU64>,
    vision_loaded: Arc<AtomicBool>,
    jobs: &mut mpsc::Receiver<Job>,
) -> Result<()> {
    send_logs_to_tracing(LogOptions::default().with_logs_enabled(false));
    let backend = LlamaBackend::init().context("llama backend")?;
    let mut next_job = None;
    let mut eager = true;
    loop {
        if !eager {
            loop {
                match jobs.blocking_recv() {
                    Some(Job::Unload { done }) => {
                        let _ = done.send(());
                    }
                    Some(Job::DropVision { .. }) => {}
                    Some(Job::Power {
                        battery: value,
                        done,
                    }) => {
                        battery.store(value, Ordering::Relaxed);
                        w_cfg.n_gpu_layers = if value { 0 } else { configured_gpu_layers };
                        let _ = done.send(());
                    }
                    Some(job) => {
                        next_job = Some(job);
                        break;
                    }
                    None => return Ok(()),
                }
            }
        }
        eager = false;
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
            .with_n_ctx(Some(
                NonZeroU32::new(w_cfg.ctx_size).unwrap_or(NonZeroU32::new(4096).unwrap()),
            ))
            .with_flash_attention_policy(flash_policy)
            .with_n_batch(w_cfg.n_batch)
            .with_n_ubatch(w_cfg.n_ubatch);

        let mut ctx = model
            .new_context(&backend, ctx_params)
            .context("llama context")?;
        tracing::info!(ms = t0.elapsed().as_millis(), "GGUF loaded");
        loaded.store(true, Ordering::Relaxed);

        let mut cached_tokens: Vec<llama_cpp_2::token::LlamaToken> = Vec::new();
        // The image projector, loaded on the first look; dropped before the model.
        let mut vision: Option<MtmdContext> = None;

        let mut unload_ack = None;
        while let Some(job) = next_job.take().or_else(|| jobs.blocking_recv()) {
            match job {
                Job::Look { image, prompt, n_predict, done } => {
                    let r = (|| -> Result<String> {
                        if vision.is_none() {
                            let proj = mmproj.as_ref().context("no image projector")?;
                            let t = std::time::Instant::now();
                            let params = MtmdContextParams { use_gpu: w_cfg.n_gpu_layers > 0, print_timings: false, ..Default::default() };
                            vision = Some(
                                MtmdContext::init_from_file(&proj.to_string_lossy(), &model, &params)
                                    .map_err(|e| anyhow::anyhow!("image projector: {e}"))?,
                            );
                            tracing::info!(ms = t.elapsed().as_millis(), path = %proj.display(), "image projector loaded");
                            vision_loaded.store(true, Ordering::Relaxed);
                        }
                        look_one(&model, &mut ctx, vision.as_mut().unwrap(), &image, &prompt, n_predict, w_cfg.n_batch)
                    })();
                    // The KV cache now holds the image, not the chat prefix.
                    cached_tokens.clear();
                    ctx.clear_kv_cache();
                    last_look.store(epoch_secs(), Ordering::Relaxed);
                    let _ = done.send(r);
                }
                Job::DropVision { since } => {
                    if vision.is_some() && last_look.load(Ordering::Relaxed) <= since {
                        vision = None;
                        vision_loaded.store(false, Ordering::Relaxed);
                        tracing::info!("image projector unloaded (idle)");
                    }
                }
                Job::Unload { done } => {
                    unload_ack = Some(done);
                    break;
                }
                Job::Power {
                    battery: value,
                    done,
                } => {
                    if battery.load(Ordering::Relaxed) == value {
                        let _ = done.send(());
                        continue;
                    }
                    battery.store(value, Ordering::Relaxed);
                    w_cfg.n_gpu_layers = if value { 0 } else { configured_gpu_layers };
                    unload_ack = Some(done);
                    break;
                }
                Job::Generate {
                    system,
                    history,
                    user,
                    n_predict,
                    temperature,
                    tokens,
                    grammar,
                    cancel,
                    done,
                } => {
                    let errors = tokens.clone();
                    let r = generate_one(
                        &model,
                        &mut ctx,
                        &mut cached_tokens,
                        w_cfg.prefix_cache,
                        w_cfg.ctx_size,
                        &system,
                        &history,
                        &user,
                        if battery.load(Ordering::Relaxed) {
                            n_predict.min(128)
                        } else {
                            n_predict
                        },
                        temperature,
                        tokens,
                        grammar.as_deref(),
                        w_cfg.n_batch,
                        cancel,
                    );
                    if let Err(err) = &r {
                        cached_tokens.clear();
                        ctx.clear_kv_cache();
                        let _ = errors.blocking_send(Err(anyhow::anyhow!("{err:#}")));
                    }
                    let _ = done.send(r);
                }
            }
        }
        drop(vision);
        vision_loaded.store(false, Ordering::Relaxed);
        drop(ctx);
        drop(model);
        loaded.store(false, Ordering::Relaxed);
        if let Some(done) = unload_ack {
            let _ = done.send(());
        } else {
            return Ok(());
        }
    }
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
    tokens_tx: mpsc::Sender<Result<String>>,
    grammar: Option<&str>,
    n_batch: u32,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let tokens_list = budget_prompt(history, ctx_size, n_predict, |remaining| {
        let prompt = chat_prompt(model, system, remaining, user)?;
        Ok(model.vocab().tokenize(prompt.as_bytes(), true, true))
    })?;
    let n_prompt = tokens_list.len();
    if n_prompt == 0 {
        anyhow::bail!("empty tokenized prompt");
    }

    let mut common_len = if prefix_cache_enabled {
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

    // Hybrid and recurrent models (Qwen 3.5) can't drop just the end of their memory; then
    // the whole prompt is processed again rather than decoding onto a stale state.
    let trimmed = common_len > 0
        && ctx.clear_kv_cache_seq(None, Some(common_len as u32), None).unwrap_or(false);
    if !trimmed {
        ctx.clear_kv_cache();
        common_len = 0;
    }
    cached_tokens.truncate(common_len);
    let capacity = (n_batch as usize).min(ctx_size as usize).max(1);
    let mut batch = LlamaBatch::new(capacity, 1);
    for chunk_start in (common_len..n_prompt).step_by(capacity) {
        if cancel.load(Ordering::Relaxed) {
            return Ok(());
        }
        batch.clear();
        let end = (chunk_start + capacity).min(n_prompt);
        for (pos, token) in tokens_list.iter().enumerate().take(end).skip(chunk_start) {
            batch.add(*token, pos as i32, &[0], pos + 1 == n_prompt)?;
        }
        ctx.decode(&mut batch).context("decode prompt batch")?;
        cached_tokens.extend_from_slice(&tokens_list[chunk_start..end]);
    }
    let mut samplers = Vec::new();
    if let Some(grammar) = grammar {
        samplers.push(
            LlamaSampler::grammar(model, grammar, "root")
                .map_err(|e| anyhow::anyhow!("invalid planning grammar: {e}"))?,
        );
    }
    if temperature <= 0.01 {
        samplers.push(LlamaSampler::greedy());
    } else {
        samplers.push(LlamaSampler::temp(temperature));
        samplers.push(LlamaSampler::dist(1234));
    }
    let mut sampler = LlamaSampler::chain(samplers, false);
    let mut decoder = UTF_8.new_decoder();
    let mut n_cur = n_prompt as i32;
    let n_max = n_cur + n_predict.max(1);

    while n_cur < n_max {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let token = sampler.sample(ctx, batch.n_tokens() - 1);
        // llama_sampler_sample already accepts the token into grammar/repetition state.
        if model.vocab().is_eog(token) {
            break;
        }
        let piece = model.vocab().token_to_piece(token, true, None);
        let mut out = String::with_capacity(
            decoder
                .max_utf8_buffer_length(piece.len())
                .unwrap_or(piece.len()),
        );
        let _ = decoder.decode_to_string(&piece, &mut out, false);
        if !out.is_empty() && tokens_tx.blocking_send(Ok(out)).is_err() {
            break;
        }
        batch.clear();
        batch.add(token, n_cur, &[0], true)?;
        ctx.decode(&mut batch).context("decode token")?;
        cached_tokens.push(token);
        n_cur += 1;
    }
    Ok(())
}

/// One answer about an image: the image and question go through the projector into the
/// context, then a short greedy answer is sampled.
fn look_one(
    model: &LlamaModel,
    ctx: &mut llama_cpp_2::context::LlamaContext<'_>,
    vision: &mut MtmdContext,
    image: &[u8],
    prompt: &str,
    n_predict: i32,
    n_batch: u32,
) -> Result<String> {
    ctx.clear_kv_cache();
    let bitmap = MtmdBitmap::from_buffer(vision, image, false).map_err(|e| anyhow::anyhow!("image: {e}"))?;
    let system = "You look at screenshots of a computer desktop and answer briefly and exactly.";
    let text = chat_prompt(model, system, &[], &format!("{}\n{prompt}", mtmd_default_marker()))?;
    let chunks = vision
        .tokenize(MtmdInputText { text, add_special: true, parse_special: true }, &[&bitmap])
        .map_err(|e| anyhow::anyhow!("tokenize image prompt: {e}"))?;
    // A projector with non-causal image attention (Gemma 4 12B) needs the whole image in one
    // micro-batch; llama.cpp aborts the process otherwise, so refuse here instead.
    if vision.decode_use_non_causal() {
        let largest = (0..chunks.len()).filter_map(|i| chunks.get(i)).filter(|c| c.chunk_type() == MtmdInputChunkType::Image).map(|c| c.n_tokens()).max().unwrap_or(0);
        anyhow::ensure!(
            largest <= ctx.n_ubatch() as usize,
            "this model's image takes {largest} tokens in one micro-batch; set llm.n_ubatch (now {}) and llm.n_batch to at least that",
            ctx.n_ubatch()
        );
    }
    let mut n_cur = chunks
        .eval_chunks(vision, ctx, 0, 0, n_batch.max(1) as i32, true)
        .map_err(|e| anyhow::anyhow!("evaluate image: {e}"))?;
    let mut sampler = LlamaSampler::greedy();
    let mut batch = LlamaBatch::new(1, 1);
    let mut decoder = UTF_8.new_decoder();
    let mut out = String::new();
    let mut idx = -1;
    for _ in 0..n_predict.max(1) {
        let token = sampler.sample(ctx, idx);
        if model.vocab().is_eog(token) {
            break;
        }
        let piece = model.vocab().token_to_piece(token, true, None);
        // decode_to_string only fills spare capacity.
        out.reserve(decoder.max_utf8_buffer_length(piece.len()).unwrap_or(piece.len()));
        let _ = decoder.decode_to_string(&piece, &mut out, false);
        batch.clear();
        batch.add(token, n_cur, &[0], true)?;
        ctx.decode(&mut batch).context("decode token")?;
        n_cur += 1;
        idx = 0;
    }
    Ok(strip_thinking(&out).to_string())
}

/// Reasoning models put their thoughts in <think>…</think>; only the answer is wanted.
fn strip_thinking(text: &str) -> &str {
    match text.rfind("</think>") {
        Some(i) => text[i + "</think>".len()..].trim(),
        None if text.trim_start().starts_with("<think>") => "",
        None => text.trim(),
    }
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
            msgs.push(
                LlamaChatMessage::new("user".into(), u.clone())
                    .map_err(|e| anyhow::anyhow!("{e}"))?,
            );
            msgs.push(
                LlamaChatMessage::new("assistant".into(), a.clone())
                    .or_else(|_| LlamaChatMessage::new("model".into(), a.clone()))
                    .map_err(|e| anyhow::anyhow!("{e}"))?,
            );
        }
        msgs.push(
            LlamaChatMessage::new("user".into(), user.into())
                .map_err(|e| anyhow::anyhow!("{e}"))?,
        );
        if let Ok(mut formatted) = model.apply_chat_template(&tmpl, &msgs, true) {
            // Models that reason in <think> blocks (Qwen 3) would speak their reasoning and
            // take seconds to start: an empty block opens the answer directly.
            if tmpl.to_str().is_ok_and(|t| t.contains("<think>")) && !formatted.trim_end().ends_with("</think>") {
                formatted.push_str("<think>\n\n</think>\n\n");
            }
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

fn budget_prompt<T>(
    history: &[(String, String)],
    ctx_size: u32,
    n_predict: i32,
    mut tokenize: impl FnMut(&[(String, String)]) -> Result<Vec<T>>,
) -> Result<Vec<T>> {
    let budget = (ctx_size as usize)
        .checked_sub(n_predict.max(1) as usize)
        .context("response allowance exceeds context")?;
    for start in 0..=history.len() {
        let tokens = tokenize(&history[start..])?;
        if tokens.len() <= budget {
            return Ok(tokens);
        }
    }
    anyhow::bail!("system and user prompt exceed context budget ({budget} tokens)")
}

#[cfg(test)]
mod tests {
    use super::gemma4_prompt;

    #[test]
    fn thinking_is_stripped() {
        use crate::strip_thinking;
        assert_eq!(strip_thinking("<think>hmm, m3 or m4</think>\nm4"), "m4");
        assert_eq!(strip_thinking("  m2 "), "m2");
        assert_eq!(strip_thinking("<think>cut off before the end"), "");
    }

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

#[cfg(test)]
mod budget_tests {
    use super::*;
    #[test]
    fn oldest_history_is_trimmed_and_errors_reach_the_caller() {
        let history = vec![
            ("old".into(), "old response".into()),
            ("recent".into(), "response".into()),
        ];
        let mut seen = Vec::new();
        let result = budget_prompt(&history, 12, 5, |history| {
            seen.push(history.first().map(|h| h.0.clone()));
            Ok(vec![0; 4 + history.len() * 3])
        })
        .unwrap();
        assert_eq!(result.len(), 7);
        assert_eq!(seen, vec![Some("old".into()), Some("recent".into())]);
        assert!(budget_prompt::<u8>(&[], 8, 5, |_| Ok(vec![0; 4])).is_err());
        let error = budget_prompt::<u8>(&history, 20, 5, |_| anyhow::bail!("tokenization failed"))
            .unwrap_err();
        assert!(error.to_string().contains("tokenization failed"));
    }
}
