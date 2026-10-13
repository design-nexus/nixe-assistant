# Nyx architecture

Nyx is a low-latency local voice-assistant runtime for Omarchy/Hyprland.
The persona name is **Nyx** (rename via `assistant.name`). The process is `omarchy-assistantd`.

This is not an Ollama replacement. It is a resident capture → STT → LLM → TTS loop
with a Unix socket for the HUD and Hyprland.

## Pipeline (phase 1)

```
hold Alt+Space
  → PipeWire capture (16 kHz S16 mono)
  → key-up ends the take (energy VAD is levels / barge-in only)
  → warm Parakeet via isolated Voxtype daemon (nyx-parakeet.service)
  → high-confidence router (desktop) or resident GGUF (llama.cpp CUDA)
  → phrase chunker
  → Kokoro ONNX (CPU on Blackwell; CUDA skipped, no sm_120 ORT kernels)
  → PipeWire playback (24 kHz F32 mono)
```

Primary metric: **user speech end → first audible response word**.

## Crates

| Crate | Role |
|---|---|
| `assistant-core` | config, `apps.toml`, `commands.toml`, state machine, events, timing, XDG paths |
| `assistant-audio` | PipeWire capture/playback, energy VAD, playback cancel |
| `assistant-inference` | `LlamaCppBackend` — in-process llama.cpp CUDA |
| `assistant-agent` | Voxtype STT, Kokoro TTS, phrase chunker, desktop router |
| `assistant-ipc` | Unix NDJSON socket |
| `assistant-tools` | typed tool specs + permission classes |
| `assistant-omarchy` | `hyprctl` / `omarchy` / Hyprland `exec_cmd` wrappers |
| `omarchy-assistantd` | daemon + `omarchy-assistant` CLI |

## State machine

`IDLE → LISTENING → TRANSCRIBING → THINKING → SPEAKING → IDLE`

Also: `EXECUTING` (desktop commands), `INTERRUPTED`, `ERROR`.

Every transition is an IPC `state` event with a wall-clock `at_ms`.

## IPC

Socket: `$XDG_RUNTIME_DIR/omarchy-assistant/assistant.sock` (0700 dir, 0600 sock).

Newline-delimited JSON. Commands from UI/Hyprland; events from the daemon.

Commands: `ptt_start`, `ptt_stop`, `cancel`, `status`, `shutdown`, `say` (skip STT), `demo` (Kokoro wav → Parakeet → LLM → speakers), `reload` (`apps.toml`, `commands.toml`, and `config.toml` without restarting models).

Events: `state`, `partial_transcript`, `transcript`, `assistant_text`, `level`, `timing`, `error`, `health`.

No localhost HTTP between our components. llama.cpp is in-process. Voxtype is a local subprocess.

## Library decisions

| Layer | Choice | Fallback |
|---|---|---|
| LLM | `llama-cpp-2` 0.1.158, feature `cuda` | persistent `llama-server` on a unix socket |
| TTS | `loqui` 0.3.0 Kokoro ONNX, feature `cuda` | skip CUDA on compute 12.* (or `kokoro-cuda-skip` stamp); else probe then CPU if infer fails / `libcudnn.so` is missing |
| Audio | official `pipewire` 0.9 | `cpal` via PipeWire |
| VAD | Silero VAD v5 ONNX CPU (sharing `ort` with Kokoro) | energy/RMS fallback |
| STT | warm Parakeet (`nyx-parakeet.service`, `voxtype-onnx-avx2`) | cold `transcribe --engine parakeet` if the daemon take is empty |

Blackwell (RTX 5080, sm_120): use `CMAKE_CUDA_ARCHITECTURES=120a-real` in the local `build.env` override.

## Voxtype boundary

F9 dictation keeps the stock Whisper daemon (`/usr/bin/voxtype`, `~/.config/voxtype`).
Nyx STT is a second, isolated Parakeet daemon (`nyx-parakeet.service`,
`XDG_RUNTIME_DIR=…/nyx-voxtype`, config `~/.config/omarchy-assistant/voxtype-parakeet.toml`).

- PTT uses `record start --no-osd --file` against that warm daemon.
- We do not change `~/.config/voxtype/config.toml` or the F9 binds.
- We do not steal F9 / Super+Ctrl+X.

## Desktop commands

High-confidence phrases live in `~/.config/omarchy-assistant/commands.toml`
(template `config/commands.toml`): lock, volume, workspaces 1–5, screenshots,
Hyprland window ops, Omarchy toggles. The router matches that file.

Launchable apps are **not** compiled in. They live in
`~/.config/omarchy-assistant/apps.toml` (template `config/apps.toml`).
`omarchy-assistant reload` or SIGHUP rereads those files and `config.toml`
(permissions, barge-in, name, VAD, LLM sampling). GGUF path and TTS device
still need a restart. GUI `omarchy launch` /
`capture` / `menu` / `exec` run through Hyprland `exec_cmd` so the window
belongs to the compositor, not the systemd daemon.

## Config

`~/.config/omarchy-assistant/config.toml` (XDG). Template: `config/config.toml`.
Apps: `~/.config/omarchy-assistant/apps.toml`.
Commands: `~/.config/omarchy-assistant/commands.toml`.

`[llm].model = "auto"` discovers GGUFs under the user model, LM Studio, and Hugging Face cache directories and prefers small voice-friendly instruct models by filename before falling back to discovered instruct models. Projection and split files are excluded.

## PTT

Default: hold **Alt+Space**. Key-up ends the utterance.
`Super+Alt+Space` remains the Omarchy Apps menu.

Talking over TTS after a short grace, or pressing Alt+Space during a reply,
cancels playback (barge-in). Phase 2: PipeWire webrtc echo-cancel
(`[barge_in] aec = true`). HUD/PTT capture the analog mic. A second stream
targets `nyx-echo-cancel` for barge-in levels. Playback goes to the analog
speakers; `settings_eq` is for music/video and is not used. A wire loop keeps
`echo-cancel-playback` unlinked from the speakers and links the analog
monitor into `echo-cancel-sink` so the canceller hears Nyx. Barge-in on the
AEC stream sits above residual TTS leak measured during grace. Matches the
session sink format; does not force 16 kHz mono.

Hands-free wake (`[wake_word] enabled`): while Idle, analog capture stays
armed. Energy VAD plus warm Parakeet. Name/aliases and prefixes live in
`config.toml`. Rest of the utterance is the ask; bare name is a follow-up
listen. Unmatched speech stays idle. Alt+Space PTT is unchanged.

## Later phases

See `docs/plan.md`: dual-model (skipped), MCP, HTTP, TensorRT,
Kokoro sm_120, F9/Voxtype/Omarchy system trees. One phase at a time.

Latest 100 LLM turns stay in RAM for follow-ups. Say "forget everything" or "start over" to clear. Not written to disk.

The daemon owns the MCP registry and approval lifecycle. Additive request/result IPC and running settings are described in [reviewed improvements](REVIEWED-IMPROVEMENTS.md).


Conversation and automation sessions are daemon-owned. After an answer, follow-up capture runs for ten seconds without a wake word. Explicit conversation mode expires after sixty idle seconds. Cancellation, mute, or stop exits the session. Push-to-talk and text requests can take over follow-up capture. Dictation tracks a Hyprland window address, pauses on focus changes, and retains failed text; terminal dictation remains a draft until explicit insertion and never presses Enter.

The pinned Playwright worker is a first-party stdio MCP server. Background DuckDuckGo lookup and visible Google tasks use separate persistent Chromium profiles. The worker exposes fixed browser operations and element references, not arbitrary JavaScript execution. Detached or navigated references fail. Search challenges pause automation for manual takeover; resuming retries the pending factual request. External MCP tools retain their separate permissions.

Managed terminal jobs preserve argument arrays, canonicalize working directories within the home directory, capture bounded output, and terminate their process group on cancellation or timeout. Generated commands/scripts require approval; explicitly configured argument arrays run directly. Task planning permits twelve calls, fifteen seconds per invocation, and 180 seconds of active processing, excluding approval waits. Each approval expires after sixty seconds and accepts spoken approve/deny or keyboard approval. Structured tool output and sources appear in Activity.
