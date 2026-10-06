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
    if args.first() == Some(&"exec") {
        let cmd = args[1..].join(" ");
        tracing::info!(%cmd, "exec via hypr");
        return hypr_exec(&cmd).await;
    }
    if should_hypr_exec(args) {
        let cmd = format!("omarchy {}", args.join(" "));
        tracing::info!(%cmd, "omarchy via hypr exec");
        return hypr_exec(&cmd).await;
    }
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

fn should_hypr_exec(args: &[&str]) -> bool {
    matches!(args.first().copied(), Some("launch") | Some("capture"))
        || matches!(args, ["theme", "switcher", ..])
}

/// Run a command through Hyprland so GUI apps belong to the compositor, not Nyx.
pub async fn hypr_exec(cmd: &str) -> Result<String> {
    let escaped = cmd.replace('\\', "\\\\").replace('"', "\\\"");
    let lua = format!(r#"hl.dispatch(hl.dsp.exec_cmd("{escaped}"))"#);
    hyprctl(&["eval", &lua]).await
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_commands_use_hypr_exec() {
        assert!(should_hypr_exec(&["launch", "terminal"]));
        assert!(should_hypr_exec(&["launch", "browser"]));
        assert!(should_hypr_exec(&["launch", "nautilus"]));
        assert!(should_hypr_exec(&["capture", "screenshot"]));
        assert!(should_hypr_exec(&["theme", "switcher"]));
        assert!(!should_hypr_exec(&["exec", "obsidian"]));
        assert!(!should_hypr_exec(&["system", "lock"]));
        assert!(!should_hypr_exec(&["audio", "output", "volume", "raise"]));
    }
}
