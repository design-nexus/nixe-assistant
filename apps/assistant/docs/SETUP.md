# Nyx setup

## Build deps (Hades / Omarchy)

- rustc / cargo
- cmake, clang, pkg-config
- CUDA toolkit (`nvcc`) matching the installed NVIDIA driver
- `libpipewire` development files (`pkg-config libpipewire-0.3`)
- Voxtype on PATH (`voxtype transcribe`)

Blackwell (RTX 5080): the workspace sets `CMAKE_CUDA_ARCHITECTURES=120a-real`
in `.cargo/config.toml` so llama.cpp MXFP4 kernels compile.

Kokoro CUDA needs `libcudnn.so` and an ONNX Runtime CUDA EP with kernels for
this GPU. On Blackwell (compute 12.0) ORT has no sm_120 kernels, so Nyx skips
CUDA TTS and uses CPU. llama.cpp CUDA does not need cuDNN. To retry Kokoro on
GPU later, delete `~/.cache/omarchy-assistant/kokoro-cuda-skip`. To put Kokoro
on GPU after kernels exist:

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
- enables the systemd **user** units (`omarchy-assistantd`, `omarchy-assistant-hud`, `nyx-parakeet`)
- appends Alt+Space PTT binds to `~/.config/hypr/bindings.lua` if they are not there

Does not touch `/usr/share/omarchy/` or `~/.config/voxtype/`.

## Run without install

```bash
RUST_LOG=info cargo run --release -p omarchy-assistantd --bin omarchy-assistantd
# another terminal:
omarchy-assistant status
omarchy-assistant say "What is two plus two?"
omarchy-assistant demo          # Kokoro → Parakeet → LLM → speakers
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

The system prompt, HUD label, and wake phrase follow this name. `{name}` in
`[assistant] prompt` is replaced at reply time. Edit the prompt in
`config.toml`, then `omarchy-assistant reload`.

## VAD

`[vad] backend = "silero"` (default) runs Silero v5 on CPU through the same
ONNX Runtime as Kokoro. `threshold` is speech probability (try `0.5`). Wake,
follow-up listen, and barge-in use it. If the ONNX fails to load, Nyx falls
back to energy/RMS. Force energy:

```toml
[vad]
backend = "energy"
threshold = 0.012
```

Then `omarchy-assistant reload`.

## Hands-free wake

Say **Hey Nyx** (or just **Nyx**) while idle, then the request. Anything after
her name is the ask. Alt+Space PTT still works. While she is listening or
speaking, **Escape** on the HUD overlay stops her. Escape is not a Hyprland
bind, so other apps keep the key when she is idle.

Wake sensitivity, hangover, and STT aliases live in
`~/.config/omarchy-assistant/config.toml` under `[wake_word]` (`phrases`,
`threshold`, `max_ms`, …). Reload with `omarchy-assistant reload`. Off:

```toml
[wake_word]
enabled = false
```

Then `omarchy-assistant reload`.

## Clock

The local GGUF has no wall clock. Nyx reads OS local time from `[clock]` in
`config.toml` (`time_phrases`, `date_phrases`, `time_format`, `date_format`,
templates). Matching phrases skip the model and speak the time; other questions
still get `{time}` and `{date}` in the system prompt. Reload after edits.

## Voice apps

Launched programs (terminal, browser, files, Spotify, …) are not compiled in.
Edit `~/.config/omarchy-assistant/apps.toml`, then `omarchy-assistant reload`
(or `systemctl --user reload omarchy-assistantd` / SIGHUP). Models stay loaded.
Shipped template: `config/apps.toml`. Use `omarchy = ["launch", "nautilus"]` or
`exec = ["env", "GSK_RENDERER=gl", "/usr/bin/strata"]`.

## Voice commands

Desktop phrases (lock, volume, workspaces, screenshots, Hyprland ops, …) live in
`~/.config/omarchy-assistant/commands.toml` (template: `config/commands.toml`).
Edit `[[command]]` phrases, `[[say]]` refusals, `forget`, `starts`, `vocab`, or
`[workspace]`, then `omarchy-assistant reload`. No rebuild.

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

## Voxtype

F9 dictation stays Whisper. Nyx STT is Parakeet in `nyx-parakeet.service`
(`voxtype-onnx-avx2`, isolated runtime). `install.sh` enables that unit.
Do not point `/usr/bin/voxtype` at ONNX unless you mean to change F9 too.

## VRAM

The daemon keeps one GGUF and Kokoro ONNX resident. Do not also park a large
Ollama/LM Studio model on the 5080 while measuring Nyx, or the numbers lie.

## PTT bind

Snippet: `scripts/hypr-bind.lua`. Default is **Alt+Space** hold-to-talk.
`Super+Alt+Space` is the Apps menu and stays that way.

## HUD

Standalone Quickshell overlay (not wired into `shell.json`). `install.sh`
enables `omarchy-assistant-hud.service`; do not also run `scripts/hud.sh`
while that unit is active.

```bash
systemctl --user status omarchy-assistant-hud
# manual, only if the unit is not enabled:
./scripts/hud.sh
```

It runs `omarchy-assistant listen` and shows state, transcript, reply, and
capture level, using the live Omarchy theme and font.
