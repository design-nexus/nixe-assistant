# Nixe Assistant HUD

Standalone Quickshell overlay. It does not go in `shell.json`.
`nixe-assistantd.service` starts the HUD and Parakeet speech services. Stopping
or restarting the daemon applies to both helpers; they do not start independently
at login. Do not also run `hud.sh` while the HUD unit is active.

```bash
systemctl --user status nixe-assistant-hud
# manual fallback if the unit is not enabled:
./scripts/hud.sh
```

`shell.qml` runs `nixe-assistant listen` and shows:

- Listening / Transcribing / Thinking / Executing / Speaking / Error
- capture level while listening
- transcript and assistant text
- Omarchy theme colors from `~/.local/state/omarchy/current/theme/colors.toml`
- Omarchy font (`monospace` alias + `[font] base-size` from shell.toml)

The HUD hides completely when the assistant is disabled or disconnected and reconnects automatically when it returns. The card hides 2.5 s after idle. Clicks pass through the overlay. While she is
busy, the overlay takes Exclusive keyboard focus so Escape cancels her without
a Hyprland bind. Idle: no grab, apps get Escape.

HUD enablement follows `settings.hud_enabled`; tool approvals show exact JSON arguments with Y/N controls and expiry. Installed assets are in `~/.local/share/nixe-assistant/quickshell`.


The HUD shows conversation/follow-up countdowns, dictation targets and retained drafts, and tool progress. Follow-up and dictation capture release keyboard focus so you can keep using the selected app. Approval prompts show exact arguments and accept Y/N or spoken approve/deny. Browser challenges can be handled with `nixe-assistant browser-takeover`, followed by `browser-resume`.
