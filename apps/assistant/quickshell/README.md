# Nyx HUD

Standalone Quickshell overlay. It does not go in `shell.json`.

```bash
./scripts/hud.sh
# or:
qs --path /home/ken/Projects/omarchy-assistant/quickshell --daemonize --no-duplicate
```

`shell.qml` runs `omarchy-assistant listen` and shows:

- Listening / Transcribing / Thinking / Executing / Speaking / Error
- transcript and assistant text

The card hides 2.5 s after idle. Clicks pass through the overlay.
