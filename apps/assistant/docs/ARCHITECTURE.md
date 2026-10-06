# Nyx architecture

Nyx is a low-latency local voice-assistant runtime for Omarchy/Hyprland.
The persona name is **Nyx** (rename via `assistant.name`). The process is `omarchy-assistantd`.

This is not an Ollama replacement. It is a resident capture → STT → LLM → TTS loop
with a Unix socket for the HUD and Hyprland.

## Pipeline (phase 1)

```
hold Alt+Space
  → PipeWire capture (16 kHz S16 mono)
  → energy VAD or key-up
  → voxtype transcribe (temp WAV)
  → resident GGUF (llama.cpp CUDA)
  → phrase chunker
  → Kokoro ONNX (CUDA)
  → PipeWire playback (24 kHz F32 mono)
```

Primary metric: **user speech end → first audible response word**.

## Crates

| Crate | Role |
|---|---|
| `assistant-core` | config, state machine, events, timing, XDG paths |
| `assistant-audio` | PipeWire capture/playback, energy VAD, playback cancel |
| `assistant-inference` | `LlamaCppBackend` — in-process llama.cpp CUDA |
| `assistant-agent` | Voxtype STT, Kokoro TTS, phrase chunker |
| `assistant-ipc` | Unix NDJSON socket |
| `assistant-tools` | typed tool specs + permission classes (stubs) |
| `assistant-omarchy` | thin `hyprctl` / `omarchy` wrappers for routed desktop commands |
| `omarchy-assistantd` | daemon + `omarchy-assistant` CLI |

## State machine

`IDLE → LISTENING → TRANSCRIBING → THINKING → SPEAKING → IDLE`

Also: `EXECUTING` (tools, later), `INTERRUPTED`, `ERROR`.

Every transition is an IPC `state` event with a wall-clock `at_ms`.

## IPC

Socket: `$XDG_RUNTIME_DIR/omarchy-assistant/assistant.sock` (0700 dir, 0600 sock).

Newline-delimited JSON. Commands from UI/Hyprland; events from the daemon.

Commands: `ptt_start`, `ptt_stop`, `cancel`, `status`, `shutdown`, `say` (skip STT), `demo` (Kokoro wav → Voxtype → LLM → speakers).

Events: `state`, `partial_transcript`, `transcript`, `assistant_text`, `level`, `timing`, `error`, `health`.

No localhost HTTP between our components. llama.cpp is in-process. Voxtype is a local subprocess.

## Library decisions

| Layer | Choice | Fallback |
|---|---|---|
| LLM | `llama-cpp-2` 0.1.158, feature `cuda` | persistent `llama-server` on a unix socket |
| TTS | `loqui` 0.3.0 Kokoro ONNX, feature `cuda` | CPU ONNX if `libcudnn.so` is missing |
| Audio | official `pipewire` 0.9 | `cpal` via PipeWire |
| VAD | energy/RMS hysteresis | Silero later (ort version clash with loqui) |
| STT | `voxtype transcribe` on a 16 kHz WAV | warm `voxtype record --file --no-osd` if cold start dominates |

Blackwell (RTX 5080, sm_120): `.cargo/config.toml` sets `CMAKE_CUDA_ARCHITECTURES=120a-real`.

## Voxtype boundary

Reuse the installed Voxtype daemon. Do not take over dictation.

- We write a temp 16 kHz mono WAV and run `voxtype transcribe <wav>`.
- We do not change `~/.config/voxtype/config.toml`.
- We do not steal F9 / Super+Ctrl+X.
- GPU Whisper/ONNX variants are a documented hint, not an automatic switch.

## Config

`~/.config/omarchy-assistant/config.toml` (XDG). Template: `config/config.toml`.

`[llm].model = "auto"` picks the first existing path among:

1. Gemma 4 E4B Instruct Q4 (LM Studio)
2. Mistral 7B Instruct Q4
3. Qwen3.5 9B Q4
4. first `*.gguf` in `~/.local/share/omarchy-assistant/models/`

## PTT

Default: hold **Alt+Space**. Key-up ends the utterance. VAD end also ends it if speech was heard.
`Super+Alt+Space` remains the Omarchy Apps menu.

## Out of phase 1

Typed Omarchy tools beyond the high-confidence router, barge-in/AEC,
dual-model cascade, wake word, MCP, TensorRT, HTTP API, Voxtype config/variant changes.

Last eight LLM turns stay in RAM for follow-ups. Say "forget everything" or "start over" to clear. Not written to disk.
