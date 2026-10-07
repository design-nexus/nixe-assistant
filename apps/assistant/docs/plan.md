# Nyx — phases

Live plan. One phase at a time. Do not edit `/usr/share/omarchy/` or
`~/.config/voxtype/` until Phase 10, and only after Ken confirms.

See `docs/ARCHITECTURE.md` for how Nyx runs. The Oct 5 session draft is
superseded by this list.

## Phase 1 — spoken loop (closing)

**Done:** Alt+Space PTT, PipeWire, warm Parakeet (F9 stays Whisper), llama.cpp
CUDA GGUF, Kokoro CPU on Blackwell, desktop router, `apps.toml`, barge-in
(energy), Quickshell HUD, 8-turn RAM memory, TTS phrase queue, empty-PTT HUD,
power-off refusal.

**This close-out:** barge-in follow-up HUD, `config.toml` hot-reload (models
stay loaded), `intent_allowed` via `assistant-tools`, capture-start and
playback-cancel traces.

First-phrase audio still waits on Kokoro for that phrase; later phrases already
overlap LLM tokens. Streaming Kokoro PCM is Phase 2+ polish if needed.

## Phase 2 — AEC (done)

PipeWire webrtc echo-cancel via `pactl load-module module-echo-cancel`
(`source_name=nyx-echo-cancel`, session sink format, typically 48 kHz stereo).
HUD and PTT capture the analog mic (AEC near-end is too quiet for the level
bar). A second stream (`Nyx-AEC`) listens on `nyx-echo-cancel` for barge-in
so TTS echo does not trip energy VAD. Playback goes straight to the analog
speakers (`settings_eq` is music/video only). Pulse `monitor_mode` does not
feed the canceller a speaker reference, so a background wire loop disconnects
`echo-cancel-playback` from the speakers and `pw-link`s the analog sink
monitor into `echo-cancel-sink` by live port names (no `pw-link -w`).
`aec_args=noise_suppression=0 voice_detection=0 gain_control=0`. Barge-in
threshold is measured from residual AEC RMS during the grace window so TTS
leak does not trip. `[barge_in] aec = true`. Unload on daemon stop.

## Phase 3 — wake word (done)

Hands-free wake plus Alt+Space PTT. `[wake_word] enabled = true`. While idle,
analog capture stays armed. Energy VAD starts a warm Parakeet take. If the
transcript begins with a configured prefix (`hey` / `ok` / `okay`) and/or a
configured name/alias (`[wake_word] phrases`, plus `assistant.name`), the rest
is the ask. Bare name starts a follow-up listen (HUD Listening). Unmatched
room speech is ignored (no HUD). PTT still works; a leading name is stripped
the same way. Disable with `enabled = false` and `omarchy-assistant reload`.
STT-as-KWS (no dedicated wake model). Escape on the HUD overlay cancels a
busy turn; it is not a Hyprland bind.

## Phase 4 — dual-model cascade (skipped)

Small fast model first, larger model when needed. Skipped while Gemma 4 E4B Q4
is fast enough. Resident GGUF stays one model.

## Phase 5 — Silero VAD (done)

Silero VAD v5 ONNX on CPU via the same `ort` 2.0 as Kokoro. Used for wake
utterance bounds, follow-up listen, and barge-in on AEC PCM. `[vad] backend =
"silero"` (threshold is speech probability). Load failure or `backend =
"energy"` keeps the RMS gate. Model: `~/.cache/omarchy-assistant/silero/silero_vad.onnx`
(downloaded on first run from `[vad] url`).

## Phase 6 — MCP / coding-agent handoff (done)

Typed MCP tools. No unrestricted shell (`permissions.shell = "deny"`).
Fast-path router detects agent handoffs ("ask claude to...", "ask agy to...",
"code <prompt>", "open coding agent"). Spawns interactive agent (`agy`,
`claude`, `codex`, `cursor`) in a dedicated terminal window on the active
workspace via Hyprland dispatch (`omarchy-launch-terminal`), speaking
aloud ("Handing off to Antigravity."). Stdio JSON-RPC 2.0 MCP client and
sandboxed typed tools (`files.read`, `git.status`, `agent.handoff`) implemented
in `assistant-tools`. Configurable via `[agent]` and `[mcp]` tables in `config.toml`.

## Phase 7 — HTTP API

Local HTTP for HUD/debug if Unix socket is not enough. Avoid remote.

## Phase 8 — TensorRT / CUDA graphs / speculative decoding

Latency after the spoken loop is solid.

## Phase 9 — Kokoro CUDA on Blackwell

Needs Linux ORT CUDA EP with sm_120 kernels. Skip stamp remains until then.
llama.cpp GGUF already uses CUDA.

## Phase 10 — F9 / Voxtype / Omarchy system trees

Only with an explicit ask. Do not auto-edit `/usr/share/omarchy/` or
`~/.config/voxtype/`.

---

## Environment (inspected 2026-10-05, host Hades)

Build a specialized low-latency local voice-assistant runtime for Omarchy/Hyprland. The assistant's name is **Nyx** (user-renameable later via config). The process is still `omarchy-assistantd`; Nyx is the persona, HUD label, and system-prompt name. This is not a general Ollama replacement.

Repo: `/home/ken/Projects/omarchy-assistant` (new). Do not edit `/usr/share/omarchy/` or the user's Voxtype dictation config.

## What this slice builds

First milestone only:

```
hold Alt+Space
  → mic capture (PipeWire)
  → speech-end (VAD)
  → Voxtype transcribe
  → resident GGUF LLM (llama.cpp CUDA)
  → Kokoro TTS
  → speakers
```

Primary metric: **user speech end → first audible response word**. Timing traces in development mode. Record every measurement in `docs/BENCHMARKS.md`.

Working audio in/out beats architectural completeness. Later phases stay sketched, not implemented.

---

## Environment (inspected 2026-10-05, host Hades)

| Component | Installed | Notes |
|---|---|---|
| GPU | RTX 5080 Laptop, 16303 MiB | compute cap 12.0 (Blackwell sm_120) |
| Driver / CUDA | nvidia-utils 615.71.09 / CUDA 13.4.2 | `nvcc` 13.4 present |
| Rust | rustc/cargo 1.99.0 | clang 23, cmake 4.4.4 |
| Omarchy | 4.0.0.r6720.g8e02fc8-1 | Hyprland 0.56.2, Quickshell 0.3.1 |
| PipeWire | 1.6.9 + WirePlumber 0.5.18 | `libpipewire-0.3` pkg-config OK |
| Voxtype | 1.1.0 (`voxtype-bin`), user systemd **enabled/active** | dictation already bound |
| Kokoro | **not installed** | we bring ONNX in-process |
| llama.cpp CLI | **not installed** | we compile via Rust bindings |
| Ollama / LM Studio | present | unused by this product |

### Voxtype (reuse, do not take over)

- Daemon is resident at pid from `$XDG_RUNTIME_DIR/voxtype/`.
- Current variant is **Whisper AVX2, CPU-only**, model `base.en`. GPU variants are installed: Whisper Vulkan (recommended by `voxtype info variants`) and ONNX CUDA 13.
- Control: `voxtype record start|stop|toggle|cancel`. Per-recording override `--file=PATH --no-osd` writes a transcript without typing and without the Voxtype OSD.
- File STT: `voxtype transcribe <wav>` (16 kHz mono WAV → stdout). Does not change the user's `output.mode = "type"` dictation setup.
- Status: `voxtype status --follow --format json`. Omarchy bar already uses this (`omarchy-voxtype-status`).
- Audio meter socket: `$XDG_RUNTIME_DIR/voxtype/audio.sock` (peak/rms/vad NDJSON via `voxtype-audio-bridge`). Not a transcript IPC.
- Voxtype already owns **F9** (PTT dictation) and **Super+Ctrl+X** (toggle dictation). Leave those alone.

Do not auto-switch the user's Voxtype GPU variant. Document a setup hint: Whisper Vulkan or ONNX CUDA 13 + a faster model will cut STT latency. Measure first.

### Models

Local GGUF already present (LM Studio): Gemma 4 E4B Q4, Mistral 7B Instruct Q4, Qwen3.5 9B Q4. Those are valid first-run candidates. We are **not** limited to them.

Nyx may download better GGUF into `~/.local/share/omarchy-assistant/models/` when a smaller/faster instruct model will cut speech-end → first-audio. Prefer latency and spoken-answer quality over leaderboard prestige. Skip reasoning/thinking models (R1, Phi-4-reasoning, etc.).

Likely first download if E4B is not the right shape: a 1–4B instruct (Qwen3 / Llama 3.2 class) for the conversational path, Q4_K_M or similar. Measure on this 5080 before making it the default.

### PTT hotkey

**Default PTT: `Alt+Space`** (hold to talk, release to stop — same pattern as Voxtype's F9).

Checked against live Hyprland binds: `Alt+Space` is free. `Super+Alt+Space` is Omarchy Apps menu and stays that way. Configurable later in `config.toml` plus `~/.config/hypr/bindings.lua`.

---

## Architecture

One resident Rust daemon. Quickshell talks to it over a Unix socket. No localhost HTTP between our components.

```
Microphone → PipeWire → VAD → SpeechToText
                                ↓
                         IntentRouter (stub: always LLM in this slice)
                                ↓
                         InferenceBackend (streaming)
                                ↓
                         phrase chunker → TextToSpeech → PipeWire → Speakers
```

UI / Hyprland / CLI send `ptt.start` / `ptt.stop` / `cancel` / `status` on the socket.

### Workspace

```
omarchy-assistant/
├── Cargo.toml                 # workspace
├── crates/
│   ├── assistant-core/        # config, state machine, event bus, timing, errors
│   ├── assistant-audio/       # PipeWire capture/playback, VAD, barge-in hooks
│   ├── assistant-inference/   # InferenceBackend + llama.cpp CUDA impl
│   ├── assistant-agent/       # conversation turn, streaming LLM→TTS glue (minimal)
│   ├── assistant-tools/       # typed tools + permissions (traits + 0–2 stubs)
│   ├── assistant-ipc/         # unix socket protocol
│   └── assistant-omarchy/     # hyprctl / omarchy CLI wrappers (thin)
├── daemon/omarchy-assistantd/
├── quickshell/                # overlay HUD (minimal after E2E audio works)
├── config/config.toml         # shipped example
├── systemd/omarchy-assistantd.service
├── scripts/
├── tests/
├── benchmarks/
└── docs/{ARCHITECTURE,BENCHMARKS,SETUP}.md
```

Edition 2021, tokio, anyhow/thiserror, serde, tracing. Match Ken's other Rust apps (vramtui/disktui): small crates, no clever framework.

### State machine (`assistant-core`)

`IDLE → LISTENING → TRANSCRIBING → THINKING → SPEAKING → IDLE`

Also: `EXECUTING` (unused until tools), `INTERRUPTED`, `ERROR`.

Every transition emits an IPC event with monotonic timestamps so the HUD and traces stay in sync.

### IPC

Socket: `$XDG_RUNTIME_DIR/omarchy-assistant/assistant.sock` (0700 dir, 0600 sock).

Newline-delimited JSON, one object per message. Commands from UI/Hyprland; events from daemon.

Commands (slice 1): `ptt.start`, `ptt.stop`, `cancel`, `status`, `shutdown`.

Events: `state`, `partial_transcript`, `transcript`, `assistant_text`, `timing`, `error`, `health`.

A tiny CLI in the daemon crate (`omarchy-assistant ptt-start|status`) is the first client. Quickshell is the second.

Avoid HTTP. llama.cpp is in-process. Voxtype is a local subprocess, not a network service.

### Config

`~/.config/omarchy-assistant/config.toml`, XDG. Ship `config/config.toml` as the template.

```toml
[assistant]
name = "Nyx"
mode = "push-to-talk"

[stt]
provider = "voxtype"

[llm]
backend = "llama.cpp"
model = "auto"   # resolve to a local GGUF, download a fast instruct if missing
n_gpu_layers = -1
ctx = 4096

[tts]
provider = "kokoro"
voice = "af_heart"

[vad]
enabled = true
backend = "silero"

[wake_word]
enabled = true

[ui]
enabled = true

[permissions]
shell = "deny"
```

---

## Library choices (verified maintained; CUDA compatibility proven at first build)

Bindings are selected. The first `cargo build --release` on this 5080 is the compatibility gate, not crates.io claims.

| Layer | Choice | Why | Fallback if the first build/run fails |
|---|---|---|---|
| LLM | **`llama-cpp-2` 0.1.158+**, feature `cuda` | Most active llama.cpp Rust binding (Sep 30 2026, ~272k downloads/mo). In-process, streaming, cancel, GPU offload. Matches "no Ollama". | Persistent `llama-server` **unix socket** (not TCP) spawned by the daemon. Same `InferenceBackend` trait. |
| TTS | **`loqui-kokoro` 0.3.0**, feature `cuda` | Kokoro-82M ONNX, CUDA EP, phrase chunking, MIT, published 2026-10-04. | `kokoro-micro` (cuda) or CPU ONNX. |
| Audio | Official **`pipewire` 0.10.1** (freedesktop) | Native capture + playback, quantum/latency control. | `cpal` via PipeWire only if the native crate fights the 1.6 ABI. |
| VAD | Silero ONNX via **`voice_activity_detector`** or **`silero` 0.7** | Spec default. Tiny, CPU, 16 kHz / 512-sample windows. | Energy/RMS gate (Voxtype's `vad.backend = energy`) as a one-file backup. |
| IPC | tokio UnixStream + serde_json | Matches Omarchy/Quickshell `Socket` + `SplitParser` style. | — |

**Blackwell note:** llama.cpp needs `sm_120` / `120a-real` under CUDA 12.8+. This box has CUDA 13.4, which is the right toolkit. If `llama-cpp-sys-2` compiles MXFP4 as plain `sm_120` and ptxas fails, set `CMAKE_CUDA_ARCHITECTURES=120a-real` in the sys crate build env and record it in ARCHITECTURE.md. Do not jump to TensorRT-LLM.

Keep the selected GGUF loaded for the life of the daemon. Same for Kokoro ONNX. Voxtype's own model stays in Voxtype's process.

### Traits (stable; impls replaceable)

```rust
trait SpeechToText {
    async fn transcribe(&self, pcm_16k_mono: &[i16], cancel: CancellationToken) -> Result<Transcript>;
}

trait InferenceBackend {
    async fn load(&mut self, spec: ModelSpec) -> Result<()>;
    fn generate_stream(&self, req: GenerateRequest) -> impl Stream<Item = TokenOrToolCall>;
    fn cancel(&self);
}

trait TextToSpeech {
    fn synthesize_stream(&self, text: impl Stream<Item = String>) -> impl Stream<Item = AudioFrame>;
    fn cancel(&self);
}
```

Partial transcripts: expose `Transcript { text, is_partial }` now. Voxtype CLI does not emit partials; fill later if Parakeet streaming or whisper eager-processing is wired.

---

## First end-to-end pipeline (what actually gets written)

### 1. Daemon skeleton
- Load config, tracing logs (`RUST_LOG` / `journalctl --user -u omarchy-assistantd`)
- Unix socket server, health, graceful SIGTERM
- State machine + event bus
- systemd user unit (install to `~/.config/systemd/user/`, not system-wide)
- `docs/ARCHITECTURE.md` written with these decisions as we lock them

### 2. PipeWire audio
- Capture 16 kHz mono from default source (configurable device name)
- Playback 24 kHz mono (Kokoro native) on default sink, small quantum
- Immediate playback cancel (flush queue) — needed later for barge-in; implement cancel now because it is cheap
- Benchmarks: capture start, playback start, playback cancel

### 3. VAD
- Silero on 32 ms frames
- Speech start / speech end / silence
- PTT mode: key-down starts capture; VAD end *or* key-up ends the utterance (key-up wins)
- Record speech-end detection latency

### 4. Voxtype STT
`VoxtypeProvider`: write a temp 16 kHz mono WAV, run `voxtype transcribe`, read stdout.

Do **not** point Voxtype at the mic and do **not** change `~/.config/voxtype/config.toml`. Dictation keeps working.

If the first benchmark shows model reload dominating (`transcribe` spawning a cold process), add `VoxtypeDaemonProvider`: `voxtype record start --file=... --no-osd` / `stop`, which uses the already-warm daemon. That path temporarily yields capture to Voxtype. Document the trade. Prefer keeping our capture.

### 5. llama.cpp CUDA
- Load configured GGUF at startup, all layers on GPU (`n_gpu_layers = -1`)
- Streaming tokens, cooperative cancel
- Short system prompt: Nyx, Omarchy desktop assistant, concise spoken answers, no markdown. Name comes from `assistant.name`.
- No tool calling in this slice
- Time: load, VRAM, TTFT, tok/s

### 6. Kokoro + phrase chunker
- Load ONNX + voice at startup
- Chunker: emit on `.?!` or ~12 words, whichever first. Short replies start immediately.
- Queue PCM into PipeWire playback
- Time: first LLM token → first audio sample

### 7. Push-to-talk wiring
- IPC `ptt.start` / `ptt.stop`
- Hyprland: `o.bind("ALT + SPACE", "Nyx PTT", "omarchy-assistant ptt-start")` and the matching release bind for `ptt-stop` — same pattern Omarchy uses for Voxtype F9
- Bind lives in **user** `~/.config/hypr/bindings.lua` via install script / documented snippet. Never patch `/usr/share/omarchy/default/hypr/`.

### 8. Timing traces
Dev log line per turn:

```
VAD end             0 ms
STT complete      +…
routing           +…   (stub)
LLM first token   +…
TTS begins        +…
audio playback    +…
TOTAL             … ms
```

Append a dated row to `docs/BENCHMARKS.md`. Empty template committed on day one.

### 9. Minimal HUD (only after audio E2E works)
Quickshell overlay in `quickshell/`, launched by the daemon or a second user unit. Connects to the unix socket.

States: Listening / Thinking / Speaking / Complete / Error. Title/persona **Nyx**. Waveform from our capture RMS, partial/final transcript, assistant text.

This is an **overlay plugin candidate** (`kinds: ["overlay"]`) later. First version can be a standalone `qs -c …` config in the repo so we do not touch `shell.json` until the socket protocol is stable.

Do not build a bar widget, settings panel, or Omarchy menu entry in this slice.

---

## Later phases (see top)

Wake word, dual-model, MCP, TensorRT, HTTP, AEC, Silero VAD, Kokoro sm_120, and
F9/Voxtype/Omarchy system edits are Phases 2–10 at the top of this file. One
phase at a time.

---

## Integration rules (Omarchy)

- Read `/usr/share/omarchy/` freely. Write only `~/.config/` and the new repo.
- Tools later wrap existing binaries (`omarchy-launch-*`, `omarchy-audio-output-volume`, `hyprctl`, `wpctl`, `omarchy-notification-send`, `omarchy-system lock`). No `sh -c` as the tool interface.
- Quickshell follows Omarchy theme tokens (`~/.local/state/omarchy/current/theme/`), same as other design-nexus plugins.
- systemd: user unit, `After=pipewire.service`, `Wants=pipewire.service`. Does not replace `voxtype.service`.

---

## Docs to create with the code

- `docs/ARCHITECTURE.md` — crate map, traits, IPC, state machine, library decisions, Voxtype boundary
- `docs/BENCHMARKS.md` — metric definitions + empty results table + how to run `cargo bench` / the timing log
- `docs/SETUP.md` — build deps (cuda, clang, pipewire), first run, Alt+Space bind, renaming Nyx, model download dir, Voxtype GPU hint, VRAM budget

---

## Implementation order (this slice)

1. Workspace + crate stubs + config/logging/IPC/state machine/systemd
2. PipeWire capture + playback + cancel + audio benches
3. Silero VAD
4. `VoxtypeProvider` (`voxtype transcribe`)
5. `llama-cpp-2` CUDA load + stream (prove sm_120 here; fallback if needed)
6. Kokoro CUDA + phrase chunker + playback
7. Glue one PTT turn; print traces; write first BENCHMARKS row
8. Hyprland `Alt+Space` bind + tiny CLI
9. Minimal Quickshell overlay talking to the socket

Stop when one spoken sentence produces one spoken reply with a measured total.

## Risks

- **llama-cpp-2 × CUDA 13.4 × sm_120 compile.** Highest risk. Budget time for the sys-crate fallback (`CMAKE_CUDA_ARCHITECTURES=120a-real` or unix-socket `llama-server`).
- **`voxtype transcribe` cold start.** If STT is hundreds of ms of model load, switch to the warm daemon `--file` path for the milestone.
- **Voxtype currently CPU Whisper `base.en`.** STT may dominate the total even when LLM/TTS are fast. Measure; then recommend a variant/model change, do not apply it silently.
- **VRAM sharing** with LM Studio / Ollama if those get started later. Assistant owns its GGUF; document ~VRAM at idle.
- **Echo / barge-in** deferred; PTT with key-up avoids most echo for the prototype (user is not talking over TTS yet).
