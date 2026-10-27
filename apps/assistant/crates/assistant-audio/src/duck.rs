//! Lower other PipeWire/Pulse playback streams while Assistant audio is queued.
//! Originals are journaled before changing a stream, so a restarted daemon can recover them.

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::PathBuf,
    sync::{
        atomic::{AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
use tokio::{process::Command, sync::watch};

#[derive(Clone, Debug)]
struct Stream {
    index: u32,
    serial: String,
    volume: Vec<u32>,
    eligible: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize)]
struct Changed {
    serial: String,
    original: Vec<u32>,
    ducked: Vec<u32>,
}

pub struct Ducker {
    stop: watch::Sender<bool>,
}

impl Drop for Ducker {
    fn drop(&mut self) {
        let _ = self.stop.send(true);
    }
}

pub fn start(queued: Arc<AtomicUsize>, gain: f32) -> Ducker {
    let (stop, rx) = watch::channel(false);
    tokio::spawn(async move {
        if let Err(err) = monitor(queued, gain, rx).await {
            tracing::warn!(%err, "external audio ducking stopped");
        }
    });
    Ducker { stop }
}

fn journal() -> PathBuf {
    assistant_core::paths::runtime_dir().join("ducked-streams.json")
}

fn parse_stream(value: &Value) -> Option<Stream> {
    let index = u32::try_from(value["index"].as_u64()?).ok()?;
    let serial = value["properties"]["object.serial"].as_str()?.to_string();
    let name = value["properties"]["application.name"].as_str().unwrap_or("");
    let node = value["properties"]["node.name"].as_str().unwrap_or("");
    let role = value["properties"]["media.role"].as_str().unwrap_or("");
    let eligible = !name.eq_ignore_ascii_case("Assistant")
        && !node.starts_with("Assistant")
        && !matches!(role.to_ascii_lowercase().as_str(), "communication" | "phone");
    let map = value["volume"].as_object()?;
    let channels = value["channel_map"].as_str()?.split(',').map(str::trim);
    let volume: Vec<u32> =
        channels.map(|channel| u32::try_from(map.get(channel)?["value"].as_u64()?).ok()).collect::<Option<_>>()?;
    (!volume.is_empty()).then_some(Stream { index, serial, volume, eligible })
}

async fn pactl(args: &[String]) -> Result<String> {
    let output =
        tokio::time::timeout(Duration::from_secs(3), Command::new("pactl").args(args).kill_on_drop(true).output()).await??;
    anyhow::ensure!(output.status.success(), "pactl: {}", String::from_utf8_lossy(&output.stderr).trim());
    anyhow::ensure!(output.stdout.len() <= 1024 * 1024, "pactl reply too large");
    Ok(String::from_utf8(output.stdout)?)
}

async fn streams() -> Result<Vec<Stream>> {
    let data: Value = serde_json::from_str(&pactl(&["-f".into(), "json".into(), "list".into(), "sink-inputs".into()]).await?)?;
    Ok(data.as_array().context("invalid sink-input list")?.iter().filter_map(parse_stream).collect())
}

async fn set_volume(index: u32, values: &[u32]) -> Result<()> {
    let mut args = vec!["set-sink-input-volume".into(), index.to_string()];
    args.extend(values.iter().map(|v| format!("0x{v:x}")));
    pactl(&args).await?;
    Ok(())
}

fn save(changed: &BTreeMap<u32, Changed>) -> Result<()> {
    let path = journal();
    std::fs::create_dir_all(path.parent().context("no runtime directory")?)?;
    if changed.is_empty() {
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        return Ok(());
    }
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let tmp = path.with_extension("json.new");
    let mut file = std::fs::OpenOptions::new().write(true).create(true).truncate(true).mode(0o600).open(&tmp)?;
    file.write_all(&serde_json::to_vec(changed)?)?;
    file.sync_all()?;
    std::fs::rename(tmp, path)?;
    Ok(())
}

fn load() -> BTreeMap<u32, Changed> {
    std::fs::read(journal()).ok().and_then(|data| serde_json::from_slice(&data).ok()).unwrap_or_default()
}

async fn restore(changed: &mut BTreeMap<u32, Changed>, current: &[Stream]) -> Result<()> {
    let mut keep = BTreeMap::new();
    for (index, state) in std::mem::take(changed) {
        let Some(stream) = current.iter().find(|s| s.index == index && s.serial == state.serial) else { continue };
        // An app or the user changed the level meanwhile: leave that newer choice alone.
        if stream.volume != state.ducked {
            continue;
        }
        if let Err(err) = set_volume(index, &state.original).await {
            tracing::warn!(index, %err, "could not restore playback stream");
            keep.insert(index, state);
        }
    }
    *changed = keep;
    save(changed)
}

async fn monitor(queued: Arc<AtomicUsize>, gain: f32, mut stop: watch::Receiver<bool>) -> Result<()> {
    let mut changed = load();
    // Recover levels from an earlier daemon crash before applying any new ducking.
    if !changed.is_empty() {
        if let Ok(current) = streams().await {
            restore(&mut changed, &current).await?;
        }
    }
    let mut empty_since = None;
    loop {
        if *stop.borrow() {
            break;
        }
        let speaking = queued.load(Ordering::Relaxed) > 0;
        if speaking {
            empty_since = None;
        } else if empty_since.is_none() {
            empty_since = Some(tokio::time::Instant::now());
        }
        let restore_now = empty_since.is_some_and(|at| at.elapsed() >= Duration::from_millis(350));
        if speaking || (restore_now && !changed.is_empty()) {
            if let Ok(current) = streams().await {
                if restore_now {
                    restore(&mut changed, &current).await?;
                    empty_since = None;
                } else {
                    changed.retain(|index, state| current.iter().any(|s| s.index == *index && s.serial == state.serial));
                    for stream in current.iter() {
                        if !stream.eligible || changed.contains_key(&stream.index) {
                            continue;
                        }
                        let ducked: Vec<u32> =
                            stream.volume.iter().map(|v| ((*v as f64 * gain as f64).round() as u32).max(1)).collect();
                        if ducked == stream.volume {
                            continue;
                        }
                        changed.insert(
                            stream.index,
                            Changed { serial: stream.serial.clone(), original: stream.volume.clone(), ducked: ducked.clone() },
                        );
                        save(&changed)?;
                        if let Err(err) = set_volume(stream.index, &ducked).await {
                            tracing::warn!(index=stream.index, %err, "could not duck playback stream");
                            changed.remove(&stream.index);
                            save(&changed)?;
                        }
                    }
                }
            }
        }
        tokio::select! {
            _ = tokio::time::sleep(Duration::from_millis(if speaking { 400 } else { 100 })) => {},
            _ = stop.changed() => {},
        }
    }
    if let Ok(current) = streams().await {
        restore(&mut changed, &current).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parses_channel_order_and_excludes_assistant() {
        let value = serde_json::json!({"index":42,"channel_map":"front-right,front-left","volume":{"front-left":{"value":65536},"front-right":{"value":32768}},"properties":{"object.serial":"88","application.name":"Music"}});
        let stream = parse_stream(&value).unwrap();
        assert_eq!(stream.volume, vec![32768, 65536]);
        assert!(stream.eligible);
        let mut assistant = value;
        assistant["properties"]["application.name"] = "Assistant".into();
        assert!(!parse_stream(&assistant).unwrap().eligible);
    }
}
