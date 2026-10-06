# Nyx — Omarchy local voice assistant

Resident, fully local, two-way voice assistant for Omarchy/Hyprland.
The process is `omarchy-assistantd`. The persona is **Nyx**.

Phase 1 milestone:

```
hold Alt+Space → speak → Voxtype → (desktop command | local GGUF) → Kokoro → hear a reply
```

Primary metric: time from finishing a sentence to hearing the first word back.

## Status

Spoken closed-loop demo is measured. Alt+Space PTT, a standalone Quickshell HUD,
and a high-confidence desktop intent router are in place.

See `docs/ARCHITECTURE.md`, `docs/SETUP.md`, `docs/BENCHMARKS.md`.

## Quick start

```bash
cargo build --release -p omarchy-assistantd
./scripts/install.sh
```

Hold **Alt+Space**, talk, release.

## Layout

```
crates/     replaceable STT / TTS / inference / audio
daemon/     omarchy-assistantd + omarchy-assistant CLI
config/     shipped config.toml
systemd/    user unit
scripts/    install, Hyprland bind snippet, HUD launcher
docs/
```
