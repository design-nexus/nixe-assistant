# Nyx setup

## Build deps (Hades / Omarchy)

- rustc / cargo
- cmake, clang, pkg-config
- CUDA toolkit (`nvcc`) matching the installed NVIDIA driver
- `libpipewire` development files (`pkg-config libpipewire-0.3`)
- Voxtype on PATH (`voxtype transcribe`)

Blackwell (RTX 5080): the workspace sets `CMAKE_CUDA_ARCHITECTURES=120a-real`
in `.cargo/config.toml` so llama.cpp MXFP4 kernels compile.

Kokoro CUDA needs `libcudnn.so`. If it is missing, Nyx falls back to CPU ONNX
for TTS. llama.cpp CUDA does not need cuDNN. To put Kokoro on GPU later:

```bash
omarchy pkg add cudnn
```

```bash
cd /home/ken/Projects/omarchy-assistant
cargo build --release -p omarchy-assistantd
```

First llama.cpp CUDA compile is slow. First Kokoro run downloads ONNX weights
into `~/.cache/omarchy-assistant/loqui/`.

## Install (user session)

```bash
./scripts/install.sh
```

This:

- installs `omarchy-assistantd` and `omarchy-assistant` to `~/.local/bin`
- writes `~/.config/omarchy-assistant/config.toml` if missing
- enables the systemd **user** unit
- appends Alt+Space PTT binds to `~/.config/hypr/bindings.lua` if they are not there

Does not touch `/usr/share/omarchy/` or `~/.config/voxtype/`.

## Run without install

```bash
RUST_LOG=info cargo run --release -p omarchy-assistantd --bin omarchy-assistantd
# another terminal:
omarchy-assistant status
omarchy-assistant say "What is two plus two?"
omarchy-assistant demo          # Kokoro → Voxtype → LLM → speakers
omarchy-assistant ptt-start     # then speak
omarchy-assistant ptt-stop
```

Logs: `journalctl --user -u omarchy-assistantd -f`

## Rename Nyx

In `~/.config/omarchy-assistant/config.toml`:

```toml
[assistant]
name = "Whatever"
```

The system prompt and HUD label follow this name. Restart the daemon.

## Voice apps

Launched programs (terminal, browser, files, Spotify, …) are not compiled in.
Edit `~/.config/omarchy-assistant/apps.toml`, then `omarchy-assistant reload`
(or `systemctl --user reload omarchy-assistantd` / SIGHUP). Models stay loaded.
Shipped template: `config/apps.toml`. Use `omarchy = ["launch", "nautilus"]` or
`exec = ["env", "GSK_RENDERER=gl", "/usr/bin/strata"]`.

## Models

`[llm].model = "auto"` uses a local GGUF if present (LM Studio copies first).
To pin one:

```toml
[llm]
model = "/path/to/instruct.Q4_K_M.gguf"
n_gpu_layers = -1
```

Drop extra GGUFs in `~/.local/share/omarchy-assistant/models/`. Prefer small
instruct models for spoken latency. Skip reasoning/thinking checkpoints.

## Voxtype GPU hint

Nyx does not change Voxtype. Dictation keeps F9. If STT dominates the timing
trace, try (yourself) a GPU variant:

```bash
voxtype info variants
```

Whisper Vulkan or ONNX CUDA 13 plus a faster model will cut `STT complete`.
Measure first.

## VRAM

The daemon keeps one GGUF and Kokoro ONNX resident. Do not also park a large
Ollama/LM Studio model on the 5080 while measuring Nyx, or the numbers lie.

## PTT bind

Snippet: `scripts/hypr-bind.lua`. Default is **Alt+Space** hold-to-talk.
`Super+Alt+Space` is the Apps menu and stays that way.

## HUD

Standalone Quickshell overlay (not wired into `shell.json`):

```bash
./scripts/hud.sh
```

It runs `omarchy-assistant listen` and shows state, transcript, and reply.
