# Nyx — Omarchy local voice assistant

Resident, fully local, two-way voice assistant for Omarchy/Hyprland.
The process is `omarchy-assistantd`. The persona is **Nyx**.

Phase 1 milestone:

```
hold Alt+Space → speak → warm Parakeet → (desktop command | local GGUF) → Kokoro → hear a reply
```

Primary metric: time from finishing a sentence to hearing the first word back.

## Status

Spoken loop is up: Alt+Space PTT, warm Parakeet STT (F9 stays Whisper), llama.cpp
CUDA GGUF, Kokoro (CPU on Blackwell), desktop router, `apps.toml`, barge-in, and
a Quickshell HUD that follows the Omarchy theme and font.

See `docs/ARCHITECTURE.md`, `docs/SETUP.md`, `docs/BENCHMARKS.md`.

## Quick start

```bash
cargo build --release -p omarchy-assistantd
./scripts/install.sh
```

Hold **Alt+Space**, talk, release.

Voice-launched apps live in `~/.config/omarchy-assistant/apps.toml` (template: `config/apps.toml`). Desktop command phrases live in `commands.toml`. Edit those files, then `omarchy-assistant reload` (or SIGHUP). No rebuild.

## Layout

```
crates/     replaceable STT / TTS / inference / audio
daemon/     omarchy-assistantd + omarchy-assistant CLI
config/     shipped config.toml, apps.toml, commands.toml, voxtype-parakeet.toml
quickshell/ standalone Nyx HUD
systemd/    assistantd, HUD, nyx-parakeet user units
scripts/    install, Hyprland bind snippet, HUD launcher
docs/
```
