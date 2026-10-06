use anyhow::{Context, Result};
use tokio::process::Command;

pub async fn hyprctl(args: &[&str]) -> Result<String> {
    let out = Command::new("hyprctl")
        .args(args)
        .output()
        .await
        .context("hyprctl")?;
    if !out.status.success() {
        anyhow::bail!(
            "hyprctl {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub async fn omarchy(args: &[&str]) -> Result<String> {
    let out = Command::new("omarchy")
        .args(args)
        .output()
        .await
        .context("omarchy")?;
    if !out.status.success() {
        anyhow::bail!(
            "omarchy {} failed: {}",
            args.join(" "),
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

/// Force the default capture source unmuted so a voice "mute mic" cannot brick Nyx.
pub async fn unmute_default_source() -> Result<()> {
    let out = Command::new("wpctl")
        .args(["set-mute", "@DEFAULT_AUDIO_SOURCE@", "0"])
        .output()
        .await
        .context("wpctl set-mute")?;
    if !out.status.success() {
        anyhow::bail!(
            "unmute mic failed: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    Ok(())
}
