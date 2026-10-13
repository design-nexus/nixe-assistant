# Assistant setup

## Build deps (Hades / Omarchy)

- rustc / cargo
- cmake, clang, pkg-config
- CUDA toolkit (`nvcc`) matching the installed NVIDIA driver
- `libpipewire` development files (`pkg-config libpipewire-0.3`)
- Voxtype on PATH (`voxtype transcribe`)

Blackwell (RTX 5080): set `CMAKE_CUDA_ARCHITECTURES=120a-real`
in `~/.config/nixe-assistant/build.env` for the installer so llama.cpp MXFP4 kernels compile.

Kokoro CUDA needs `libcudnn.so` and an ONNX Runtime CUDA EP with kernels for
this GPU. On Blackwell (compute 12.0) ORT has no sm_120 kernels, so the assistant skips
CUDA TTS and uses CPU. llama.cpp CUDA does not need cuDNN. To retry Kokoro on
GPU later, delete `~/.cache/nixe-assistant/kokoro-cuda-skip`. To put Kokoro
on GPU after kernels exist:

```bash
omarchy pkg add cudnn
```

```bash
cd /path/to/nixe-assistant
cargo build --release -p nixe-assistantd
```

First llama.cpp CUDA compile is slow. First Kokoro run downloads ONNX weights
into `~/.cache/nixe-assistant/loqui/`.

## Install (user session)

```bash
./scripts/install.sh
```

This:

- installs `nixe-assistantd` and `nixe-assistant` to `~/.local/bin`
- writes `~/.config/nixe-assistant/config.toml` if missing
- enables the systemd **user** units (`nixe-assistantd`, `nixe-assistant-hud`, `nixe-assistant-stt`)
- appends Alt+Space PTT binds to `~/.config/hypr/bindings.lua` if they are not there

Does not touch `/usr/share/omarchy/` or `~/.config/voxtype/`.

## Run without install

```bash
RUST_LOG=info cargo run --release -p nixe-assistantd --bin nixe-assistantd
# another terminal:
nixe-assistant status
nixe-assistant say "What is two plus two?"
nixe-assistant demo          # Kokoro → Parakeet → LLM → speakers
nixe-assistant ptt-start     # then speak
nixe-assistant ptt-stop
```

Logs: `journalctl --user -u nixe-assistantd -f`

## Rename the assistant

In `~/.config/nixe-assistant/config.toml`:

```toml
[assistant]
name = "Whatever"
```

The system prompt, HUD label, and wake phrase follow this name. `{name}` in
`[assistant] prompt` is replaced at reply time. Edit the prompt in
`config.toml`, then `nixe-assistant reload`.

## VAD

`[vad] backend = "silero"` (default) runs Silero v5 on CPU through the same
ONNX Runtime as Kokoro. `threshold` is speech probability (try `0.5`). Wake,
follow-up listen, and barge-in use it. If the ONNX fails to load, the assistant
falls back to energy/RMS. Force energy:

```toml
[vad]
backend = "energy"
energy_threshold = 0.012
```

Then `nixe-assistant reload`.

## Hands-free wake

Say a wake phrase of the current persona (e.g. **Hey Echo**, or just **Echo**)
while idle, then the request. Anything after the name is the ask. Alt+Space PTT
still works. While the assistant is listening or speaking, **Escape** on the HUD
overlay stops it. Escape is not a Hyprland bind, so other apps keep the key when
the assistant is idle.

Wake sensitivity, hangover, and STT aliases live in
`~/.config/nixe-assistant/config.toml` under `[wake_word]` (`phrases`,
`threshold`, `max_ms`, …). Reload with `nixe-assistant reload`. Off:

```toml
[wake_word]
enabled = false
```

Then `nixe-assistant reload`.

## Clock

The local GGUF has no wall clock. The assistant reads OS local time from `[clock]` in
`config.toml` (`time_phrases`, `date_phrases`, `time_format`, `date_format`,
templates). Matching phrases skip the model and speak the time; other questions
still get `{time}` and `{date}` in the system prompt. Reload after edits.

## Voice apps

Launched programs (terminal, browser, files, Spotify, …) are not compiled in.
Edit `~/.config/nixe-assistant/apps.toml`, then `nixe-assistant reload`
(or `systemctl --user reload nixe-assistantd` / SIGHUP). Models stay loaded.
Shipped template: `config/apps.toml`. Use `omarchy = ["launch", "nautilus"]` or
`exec = ["env", "GSK_RENDERER=gl", "/usr/bin/strata"]`.

## Voice commands

Desktop phrases (lock, volume, workspaces, screenshots, Hyprland ops, …) live in
`~/.config/nixe-assistant/commands.toml` (template: `config/commands.toml`).
Edit `[[command]]` phrases, `[[say]]` refusals, `forget`, `starts`, `vocab`, or
`[workspace]`, then `nixe-assistant reload`. No rebuild.

## Models

`[llm].model = "auto"` uses a local GGUF if present (LM Studio copies first).
To pin one:

```toml
[llm]
model = "/path/to/instruct.Q4_K_M.gguf"
n_gpu_layers = -1
```

Drop extra GGUFs in `~/.local/share/nixe-assistant/models/`. Prefer small
instruct models for spoken latency. Skip reasoning/thinking checkpoints.

## Voxtype

F9 dictation stays Whisper. Assistant STT is Parakeet in `nixe-assistant-stt.service`
(`voxtype-onnx-avx2`, isolated runtime). `install.sh` enables that unit.
Do not point `/usr/bin/voxtype` at ONNX unless you mean to change F9 too.

## VRAM

The daemon keeps one GGUF and Kokoro ONNX resident. Do not also park a large
Ollama/LM Studio model on the 5080 while measuring the assistant, or the numbers lie.

## PTT bind

Snippet: `scripts/hypr-bind.lua`. Default is **Alt+Space** hold-to-talk.
`Super+Alt+Space` is the Apps menu and stays that way.

## HUD

Standalone Quickshell overlay (not wired into `shell.json`). `install.sh`
enables `nixe-assistant-hud.service`; do not also run `scripts/hud.sh`
while that unit is active.

```bash
systemctl --user status nixe-assistant-hud
# manual, only if the unit is not enabled:
./scripts/hud.sh
```

It runs `nixe-assistant listen` and shows state, transcript, reply, and
capture level, using the live Omarchy theme and font.

See [reviewed improvements](REVIEWED-IMPROVEMENTS.md) for draft/apply semantics, all new TUI features, MCP permissions, token-authenticated HTTP and portable installation.


Browser runtime: install Node 18+ and npm, then run `scripts/install.sh`. The installer uses the lockfile, installs matching Playwright Chromium under the assistant cache, and bundles the worker under the assistant data directory. `[browser] node` can override the installed `nixe-assistant-node` wrapper; `worker = "auto"` discovers the installed worker. `[browser] weather_location` is optional. Browser automation operates in an assistant-owned profile; it does not import Firefox cookies. DuckDuckGo or Google may require manual challenge completion through browser takeover.

`[conversation] follow_up_secs = 10` and `idle_secs = 60` control listening windows. `[dictation] idle_secs = 120` bounds dictation silence. `[terminal] timeout_secs = 120` supports 1–600 seconds. Add trusted commands as `[terminal.commands]` entries, for example `list-projects = ["ls", "-la", "/home/your-user/Projects"]`; working directories stay inside the home directory.


`[location] enabled = true` enables on-demand approximate public-IP geolocation through ipapi.co, with ipwho.is as a fallback. `city = ""` chooses automatic detection; set a city to use a local override without network lookup. Optional `latitude` and `longitude` must be supplied together with a city. `cache_minutes = 60` controls the transient cache (1–1440). No background polling, GPS access, or IP response persistence occurs. Location can be disabled through voice, CLI, or TUI. Explicit places in a request always take precedence; existing `[browser] weather_location` remains a weather-specific override.

Weather requests use Open-Meteo over HTTPS, including its geocoding endpoint for named cities. Current replies include the location, units, source, and local data time. US locations always use Fahrenheit/mph, including generated tool calls. Outside the US, Celsius requests use Celsius/kmh; otherwise the default is Fahrenheit/mph. Lookups validate HTTP status, bound request time and response size, reject missing or stale current data, and support cancellation. `weather.lookup` is available in the tool registry. A named city remains usable when automatic location is disabled.

### DuckDuckGo MCP factual search

Run `scripts/install-search.sh` to install the pinned AstroCorp server and its locked Node dependencies under the assistant data directory. Node 22+ is required. Then run `nixe-assistant search-provider duckduckgo` and restart the daemon. `search-provider show` reports the saved provider; `search-provider builtin` switches back after a restart. This adds the stdio server and allows only `duckduckgo.search`, retaining existing MCP permissions and servers. The server is external; its filesystem/network access is governed by its process environment.

`[browser] search_tool = "duckduckgo.search"` directs ordinary factual queries to that registered tool. A custom provider must accept `query` and return structured `results` containing `title`, `url`, optional `snippet`, `text`, and `published_at`, with `retrieved_at` at the top level. Weather still uses Open-Meteo; visible page requests still use Google. Extracted content is untrusted. Challenges are reported honestly; no CAPTCHA is solved automatically. Search citations are retained for “open that source.”

See `search/README.md` for the pinned revision and local patch. Full-content output is bounded, requests have timeouts/cancellation, and fetched page bodies are limited before parsing.
