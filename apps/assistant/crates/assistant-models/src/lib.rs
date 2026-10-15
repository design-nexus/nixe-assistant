//! The LLM catalogue shared by the TUI and the CLI (and through it, Nixe Settings): curated
//! models, Hugging Face search, and the streaming GGUF downloader.

use anyhow::{Context, Result};
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Instant;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio::sync::watch;

#[derive(Clone, Debug, Serialize)]
pub struct CuratedModel {
    pub name: &'static str,
    pub description: &'static str,
    pub size_str: &'static str,
    pub filename: &'static str,
    pub download_url: &'static str,
    pub hf_repo: &'static str,
    /// The image projector in the same repository (vision), if there is one.
    pub mmproj: Option<&'static str>,
    /// Model plus projector, roughly what it takes in VRAM before the context.
    pub vram_gb: f32,
}

const HF: &str = "https://huggingface.co/";

pub const CURATED_MODELS: &[CuratedModel] = &[
    CuratedModel {
        name: "Gemma 4 E4B (Q4_K_M)",
        description: "The default: fast spoken answers and tool planning, with vision.",
        size_str: "5.3 GB",
        filename: "gemma-4-E4B-it-Q4_K_M.gguf",
        download_url: "https://huggingface.co/lmstudio-community/gemma-4-E4B-it-GGUF/resolve/main/gemma-4-E4B-it-Q4_K_M.gguf",
        hf_repo: "lmstudio-community/gemma-4-E4B-it-GGUF",
        mmproj: Some("mmproj-gemma-4-E4B-it-BF16.gguf"),
        vram_gb: 6.3,
    },
    CuratedModel {
        name: "Gemma 4 E2B (Q4_K_M)",
        description: "Smaller and lighter on VRAM and battery; weaker at multi-step tasks. Vision.",
        size_str: "3.4 GB",
        filename: "gemma-4-E2B-it-Q4_K_M.gguf",
        download_url: "https://huggingface.co/lmstudio-community/gemma-4-E2B-it-GGUF/resolve/main/gemma-4-E2B-it-Q4_K_M.gguf",
        hf_repo: "lmstudio-community/gemma-4-E2B-it-GGUF",
        mmproj: Some("mmproj-gemma-4-E2B-it-BF16.gguf"),
        vram_gb: 4.4,
    },
    CuratedModel {
        name: "Qwen 3.5 9B (Q4_K_M)",
        description: "Larger general model, strong at tool use in its own format; slower here. Vision.",
        size_str: "5.6 GB",
        filename: "Qwen3.5-9B-Q4_K_M.gguf",
        download_url: "https://huggingface.co/lmstudio-community/Qwen3.5-9B-GGUF/resolve/main/Qwen3.5-9B-Q4_K_M.gguf",
        hf_repo: "lmstudio-community/Qwen3.5-9B-GGUF",
        mmproj: Some("mmproj-Qwen3.5-9B-BF16.gguf"),
        vram_gb: 6.6,
    },
    CuratedModel {
        name: "Gemma 4 12B QAT (Q4_0)",
        description: "Bigger Gemma for better answers when VRAM allows. Vision.",
        size_str: "7.0 GB",
        filename: "gemma-4-12B-it-QAT-Q4_0.gguf",
        download_url: "https://huggingface.co/lmstudio-community/gemma-4-12B-it-QAT-GGUF/resolve/main/gemma-4-12B-it-QAT-Q4_0.gguf",
        hf_repo: "lmstudio-community/gemma-4-12B-it-QAT-GGUF",
        mmproj: Some("mmproj-gemma-4-12B-it-QAT-BF16.gguf"),
        vram_gb: 7.2,
    },
    CuratedModel {
        name: "GLM-4.6V Flash (Q4_K_M)",
        description: "Vision model trained for screens; best at pointing at things (computer use).",
        size_str: "6.2 GB",
        filename: "GLM-4.6V-Flash-Q4_K_M.gguf",
        download_url: "https://huggingface.co/lmstudio-community/GLM-4.6V-Flash-GGUF/resolve/main/GLM-4.6V-Flash-Q4_K_M.gguf",
        hf_repo: "lmstudio-community/GLM-4.6V-Flash-GGUF",
        mmproj: Some("mmproj-GLM-4.6V-Flash-F16.gguf"),
        vram_gb: 8.0,
    },
];

impl CuratedModel {
    /// Where the image projector downloads from.
    pub fn mmproj_url(&self) -> Option<String> {
        self.mmproj.map(|f| format!("{HF}{}/resolve/main/{f}", self.hf_repo))
    }
}

#[derive(Clone, Debug, Default)]
#[allow(dead_code)]
pub struct DownloadStatus {
    pub state: DownloadState,
    pub is_downloading: bool,
    pub filename: String,
    pub downloaded_bytes: u64,
    pub total_bytes: u64,
    pub speed_bytes_sec: u64,
    pub progress_frac: f32,
    pub error: Option<String>,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct HfModelEntry {
    pub id: String,
    #[serde(default)]
    pub downloads: u64,
    #[serde(default)]
    pub likes: u64,
}

pub async fn search_huggingface(query: &str) -> Result<Vec<HfModelEntry>> {
    let clean_query = query.trim();
    if clean_query.is_empty() {
        return Ok(Vec::new());
    }
    let url = format!(
        "https://huggingface.co/api/models?search={}&filter=gguf&sort=downloads&direction=-1&limit=15",
        urlencoding_encode(clean_query)
    );
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(6))
        .user_agent("nixe-assistant")
        .build()?;
    let resp = client.get(&url).send().await?.error_for_status()?;
    let items: Vec<HfModelEntry> = resp.json().await?;
    Ok(items)
}

fn urlencoding_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b == b'-' || b == b'_' || b == b'.' || b == b'~' {
            out.push(b as char);
        } else {
            out.push_str(&format!("%{:02X}", b));
        }
    }
    out
}

pub fn is_curated_installed(model: &CuratedModel, installed: &[PathBuf]) -> bool {
    installed.iter().any(|p| {
        p.file_name()
            .and_then(|f| f.to_str())
            .map(|name| name.eq_ignore_ascii_case(model.filename))
            .unwrap_or(false)
    })
}

pub fn format_bytes(bytes: u64) -> String {
    const GIB: u64 = 1024 * 1024 * 1024;
    const MIB: u64 = 1024 * 1024;
    const KIB: u64 = 1024;
    if bytes >= GIB {
        format!("{:.2} GB", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1} MB", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.0} KB", bytes as f64 / KIB as f64)
    } else {
        format!("{} B", bytes)
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum DownloadState {
    #[default]
    Idle,
    Connecting,
    Downloading,
    Validating,
    Installed,
    Cancelled,
    Failed,
}

/// A repository's GGUF files with their sizes (bytes), and whether each is a projector.
pub async fn hf_files_sized(repo: &str) -> Result<Vec<serde_json::Value>> {
    let client = reqwest::Client::builder().timeout(std::time::Duration::from_secs(10)).user_agent("nixe-assistant").build()?;
    let value: serde_json::Value = client
        .get(format!("https://huggingface.co/api/models/{repo}?blobs=true"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let mut files: Vec<serde_json::Value> = value["siblings"]
        .as_array()
        .context("missing repository file listing")?
        .iter()
        .filter_map(|v| {
            let name = v["rfilename"].as_str()?;
            (name.ends_with(".gguf") && !split_model(name)).then(|| {
                serde_json::json!({
                    "name": name, "bytes": v["size"].as_u64(), "projector": name.starts_with("mmproj"),
                    "url": format!("https://huggingface.co/{repo}/resolve/main/{name}"),
                })
            })
        })
        .collect();
    files.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    Ok(files)
}

pub async fn hf_files(repo: &str) -> Result<Vec<String>> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()?;
    let value: serde_json::Value = client
        .get(format!("https://huggingface.co/api/models/{repo}"))
        .send()
        .await?
        .error_for_status()?
        .json()
        .await?;
    let mut files = value["siblings"]
        .as_array()
        .context("missing repository file listing")?
        .iter()
        .filter_map(|v| v["rfilename"].as_str())
        .filter(|name| name.ends_with(".gguf"))
        .map(str::to_string)
        .collect::<Vec<_>>();
    files.sort();
    Ok(files)
}
pub fn split_model(name: &str) -> bool {
    name.contains("-of-") || name.contains(".part")
}
pub fn download_target(url: &str) -> Result<(String, PathBuf)> {
    let url = reqwest::Url::parse(url)?;
    if !["http", "https"].contains(&url.scheme()) {
        anyhow::bail!("download requires HTTP(S)");
    }
    let name = url
        .path_segments()
        .and_then(|mut s| s.next_back())
        .context("URL has no filename")?;
    if !name.ends_with(".gguf") || name.contains('%') || name.is_empty() || split_model(name) {
        anyhow::bail!("URL must identify a single .gguf file; split models are unsupported");
    }
    Ok((
        url.to_string(),
        assistant_core::paths::models_dir().join(name),
    ))
}
async fn cancellable<T>(
    future: impl std::future::Future<Output = Result<T>>,
    cancel: &AtomicBool,
) -> Result<T> {
    tokio::pin!(future);
    let mut tick = tokio::time::interval(std::time::Duration::from_millis(50));
    loop {
        tokio::select! {
            biased;
            _ = tick.tick() => if cancel.load(Ordering::SeqCst) {anyhow::bail!("download cancelled");},
            result = &mut future => return result,
        }
    }
}
pub fn validate_header(bytes: &[u8]) -> Result<()> {
    if bytes.len() < 24 || &bytes[..4] != b"GGUF" {
        anyhow::bail!("response is not GGUF");
    }
    let version = u32::from_le_bytes(bytes[4..8].try_into()?);
    let tensors = u64::from_le_bytes(bytes[8..16].try_into()?);
    let metadata = u64::from_le_bytes(bytes[16..24].try_into()?);
    if ![2, 3].contains(&version)
        || tensors == 0
        || tensors > 1_000_000
        || metadata == 0
        || metadata > 1_000_000
    {
        anyhow::bail!("invalid GGUF header or unsupported version");
    }
    Ok(())
}
pub async fn download_file(
    url: &str,
    target_path: PathBuf,
    tx: watch::Sender<DownloadStatus>,
    cancel: Arc<AtomicBool>,
) -> Result<()> {
    let filename = target_path
        .file_name()
        .and_then(|f| f.to_str())
        .context("invalid filename")?
        .to_string();
    if split_model(&filename) {
        anyhow::bail!("split GGUF models are unsupported");
    }
    let mut status = DownloadStatus {
        state: DownloadState::Connecting,
        is_downloading: true,
        filename,
        ..Default::default()
    };
    tx.send_replace(status.clone());
    let result = async {
        if target_path.exists() {
            anyhow::bail!("model already exists");
        }
        let client = reqwest::Client::builder()
            .connect_timeout(std::time::Duration::from_secs(10))
            .read_timeout(std::time::Duration::from_secs(15))
            .user_agent("nixe-assistant")
            .build()?;
        let response = cancellable(
            async { Ok(client.get(url).send().await?.error_for_status()?) },
            &cancel,
        )
        .await?;
        status.total_bytes = response.content_length().unwrap_or(0);
        let parent = target_path.parent().context("missing target directory")?;
        fs::create_dir_all(parent).await?;
        // TempPath removes incomplete data on every return path, including dropped tasks.
        let temp = tempfile::NamedTempFile::new_in(parent)?.into_temp_path();
        let mut file = File::create(&temp).await?;
        let mut stream = response.bytes_stream();
        let mut header = Vec::new();
        let began = Instant::now();
        status.state = DownloadState::Downloading;
        tx.send_replace(status.clone());
        while let Some(chunk) =
            cancellable(async { Ok(stream.next().await.transpose()?) }, &cancel).await?
        {
            if header.len() < 24 {
                header.extend(chunk.iter().take(24 - header.len()));
                if header.len() == 24 {
                    validate_header(&header)?;
                }
            }
            file.write_all(&chunk).await?;
            status.downloaded_bytes += chunk.len() as u64;
            status.speed_bytes_sec =
                (status.downloaded_bytes as f64 / began.elapsed().as_secs_f64().max(0.001)) as u64;
            status.progress_frac = if status.total_bytes > 0 {
                (status.downloaded_bytes as f32 / status.total_bytes as f32).min(1.0)
            } else {
                0.0
            };
            tx.send_replace(status.clone());
        }
        status.state = DownloadState::Validating;
        tx.send_replace(status.clone());
        validate_header(&header)?;
        if status.downloaded_bytes <= 24
            || (status.total_bytes > 0 && status.downloaded_bytes != status.total_bytes)
        {
            anyhow::bail!("incomplete GGUF download");
        }
        file.sync_all().await?;
        drop(file);
        let validation_path = temp.to_path_buf();
        tokio::task::spawn_blocking(move || validate_file(&validation_path)).await??;
        if cancel.load(Ordering::SeqCst) {
            anyhow::bail!("download cancelled");
        }
        temp.persist_noclobber(&target_path)
            .map_err(|e| e.error)
            .context("install model atomically")?;
        Ok::<_, anyhow::Error>(())
    }
    .await;
    status.is_downloading = false;
    status.state = match &result {
        Ok(()) => DownloadState::Installed,
        Err(_) if cancel.load(Ordering::SeqCst) => DownloadState::Cancelled,
        Err(_) => DownloadState::Failed,
    };
    status.error = result.as_ref().err().map(ToString::to_string);
    if result.is_ok() {
        status.progress_frac = 1.0;
    }
    tx.send_replace(status);
    result
}

/// Validate GGUF metadata, tensor directory and data bounds without loading weights.
fn validate_file(path: &std::path::Path) -> Result<()> {
    use std::io::{Read, Seek, SeekFrom};
    struct Parser {
        file: std::fs::File,
        len: u64,
        values: u64,
    }
    impl Parser {
        fn read<const N: usize>(&mut self) -> Result<[u8; N]> {
            let mut bytes = [0; N];
            self.file.read_exact(&mut bytes)?;
            Ok(bytes)
        }
        fn u32(&mut self) -> Result<u32> {
            Ok(u32::from_le_bytes(self.read()?))
        }
        fn u64(&mut self) -> Result<u64> {
            Ok(u64::from_le_bytes(self.read()?))
        }
        fn skip(&mut self, n: u64) -> Result<()> {
            let pos = self
                .file
                .stream_position()?
                .checked_add(n)
                .context("GGUF offset overflow")?;
            if pos > self.len {
                anyhow::bail!("truncated GGUF metadata");
            }
            self.file.seek(SeekFrom::Start(pos))?;
            Ok(())
        }
        fn string(&mut self) -> Result<String> {
            let len = self.u64()?;
            if len > 1024 * 1024 {
                anyhow::bail!("GGUF key too long");
            }
            let mut bytes = vec![0; len as usize];
            self.file.read_exact(&mut bytes)?;
            Ok(String::from_utf8(bytes)?)
        }
        fn value(&mut self, kind: u32, depth: u8) -> Result<()> {
            self.values += 1;
            if self.values > 10_000_000 || depth > 8 {
                anyhow::bail!("GGUF metadata limit exceeded");
            }
            match kind {
                0 | 1 | 7 => self.skip(1),
                2 | 3 => self.skip(2),
                4..=6 => self.skip(4),
                10..=12 => self.skip(8),
                8 => {
                    let len = self.u64()?;
                    self.skip(len)
                }
                9 => {
                    let subtype = self.u32()?;
                    let count = self.u64()?;
                    if count > 10_000_000 {
                        anyhow::bail!("GGUF array too large");
                    }
                    for _ in 0..count {
                        self.value(subtype, depth + 1)?;
                    }
                    Ok(())
                }
                _ => anyhow::bail!("invalid GGUF metadata type"),
            }
        }
    }
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    let mut p = Parser {
        file,
        len,
        values: 0,
    };
    let header = p.read::<24>()?;
    validate_header(&header)?;
    let tensors = u64::from_le_bytes(header[8..16].try_into()?);
    let metadata = u64::from_le_bytes(header[16..24].try_into()?);
    let mut alignment = 32u64;
    for _ in 0..metadata {
        let key = p.string()?;
        let kind = p.u32()?;
        if key == "general.alignment" && kind == 4 {
            alignment = p.u32()? as u64;
            if alignment == 0 || alignment > 4096 || !alignment.is_power_of_two() {
                anyhow::bail!("invalid GGUF alignment");
            }
        } else if key == "split.count" {
            let count = match kind {
                2 => u16::from_le_bytes(p.read()?) as u64,
                4 => p.u32()? as u64,
                _ => anyhow::bail!("invalid split metadata"),
            };
            if count > 1 {
                anyhow::bail!("split GGUF models are unsupported");
            }
        } else {
            p.value(kind, 0)?;
        }
    }
    let mut extents = Vec::new();
    for _ in 0..tensors {
        let _name = p.string()?;
        let dims = p.u32()?;
        if dims == 0 || dims > 4 {
            anyhow::bail!("invalid tensor dimensions");
        }
        let mut elements = 1u64;
        for _ in 0..dims {
            let size = p.u64()?;
            if size == 0 {
                anyhow::bail!("empty tensor");
            }
            elements = elements.checked_mul(size).context("tensor size overflow")?;
        }
        let kind = p.u32()?;
        let offset = p.u64()?;
        let (block, bytes) = match kind {
            0 => (1, 4),
            1 => (1, 2),
            2 => (32, 18),
            3 => (32, 20),
            6 => (32, 22),
            7 => (32, 24),
            8 => (32, 34),
            9 => (32, 40),
            10 => (256, 84),
            11 => (256, 110),
            12 => (256, 144),
            13 => (256, 176),
            14 => (256, 210),
            15 => (256, 292),
            _ => (256, 1),
        };
        let size = elements
            .div_ceil(block)
            .checked_mul(bytes)
            .context("tensor byte size overflow")?;
        if !offset.is_multiple_of(alignment) {
            anyhow::bail!("misaligned tensor");
        }
        extents.push(offset.checked_add(size).context("tensor offset overflow")?);
    }
    let pos = p.file.stream_position()?;
    let start = pos.div_ceil(alignment) * alignment;
    if extents
        .into_iter()
        .any(|end| start.checked_add(end).is_none_or(|end| end > len))
    {
        anyhow::bail!("truncated GGUF tensor data");
    }
    Ok(())
}

#[cfg(test)]
mod failure_tests {
    use super::*;
    async fn http_fixture(response: Option<Vec<u8>>) -> String {
        use tokio::io::AsyncReadExt;
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut buf = [0; 2048];
            let _ = stream.read(&mut buf).await;
            if let Some(bytes) = response {
                let _ = stream.write_all(&bytes).await;
            } else {
                tokio::time::sleep(std::time::Duration::from_secs(2)).await;
            }
        });
        format!("http://{addr}/model.gguf")
    }
    #[tokio::test]
    async fn invalid_http_and_stalled_downloads_do_not_install() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("model.gguf");
        for response in [
            b"HTTP/1.1 404 Not Found\r\nContent-Length: 4\r\n\r\nnope".to_vec(),
            b"HTTP/1.1 200 OK\r\nContent-Length: 26\r\n\r\n<html>not a model!!!</html>".to_vec(),
        ] {
            let url = http_fixture(Some(response)).await;
            let (tx, rx) = watch::channel(DownloadStatus::default());
            assert!(
                download_file(&url, target.clone(), tx, Arc::new(AtomicBool::new(false)))
                    .await
                    .is_err()
            );
            assert_eq!(rx.borrow().state, DownloadState::Failed);
            assert!(!target.exists());
            assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0);
        }
        let url = http_fixture(None).await;
        let (tx, rx) = watch::channel(DownloadStatus::default());
        let cancel = Arc::new(AtomicBool::new(false));
        let cancellation = cancel.clone();
        let download_path = target.clone();
        let job = tokio::spawn(async move { download_file(&url, download_path, tx, cancel).await });
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        cancellation.store(true, Ordering::SeqCst);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(500), job)
                .await
                .unwrap()
                .unwrap()
                .is_err()
        );
        assert_eq!(rx.borrow().state, DownloadState::Cancelled);
        assert!(!target.exists());
        let mut header = b"GGUF".to_vec();
        header.extend(3u32.to_le_bytes());
        header.extend(1u64.to_le_bytes());
        header.extend(1u64.to_le_bytes());
        assert!(validate_header(&header).is_ok());
        std::fs::write(&target, &header).unwrap();
        assert!(validate_file(&target).is_err());
        assert!(download_target("https://host/model-00001-of-00002.gguf").is_err());
    }
}
