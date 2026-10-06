# Nyx HUD

Standalone Quickshell overlay. It does not go in `shell.json`.
`install.sh` enables `omarchy-assistant-hud.service`. Use that unit; do not
also run `hud.sh` while it is active (`hud.sh` exits if the unit is running).

```bash
systemctl --user status omarchy-assistant-hud
# manual fallback if the unit is not enabled:
./scripts/hud.sh
```

`shell.qml` runs `omarchy-assistant listen` and shows:

- Listening / Transcribing / Thinking / Executing / Speaking / Error / Offline
- capture level while listening
- transcript and assistant text
- Omarchy theme colors from `~/.local/state/omarchy/current/theme/colors.toml`
- Omarchy font (`monospace` alias + `[font] base-size` from shell.toml)

The card hides 2.5 s after idle. Clicks pass through the overlay. While she is
busy, the overlay takes Exclusive keyboard focus so Escape cancels her without
a Hyprland bind. Idle: no grab, apps get Escape.
