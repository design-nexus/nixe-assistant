# Nixe Assistant — Local Voice Assistant

Resident voice assistant for Omarchy/Hyprland.
The process is `nixe-assistantd`. Custom personas can be configured via settings.

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
cargo build --release -p nixe-assistantd
./scripts/install.sh
```

Hold **Alt+Space**, talk, release. **Super+Shift+Escape** stops everything the assistant is doing.

Voice-launched apps live in `~/.config/nixe-assistant/apps.toml` (template: `config/apps.toml`). Desktop command phrases live in `commands.toml`. Edit those files, then `nixe-assistant reload` (or SIGHUP). No rebuild.

See [settings, MCP, diagnostics and installation](docs/REVIEWED-IMPROVEMENTS.md) for the reviewed improvements and new controls.

## Layout

```
crates/     replaceable STT / TTS / inference / audio
daemon/     nixe-assistantd + nixe-assistant CLI
config/     shipped config.toml, apps.toml, commands.toml, voxtype-parakeet.toml
quickshell/ standalone Assistant HUD
systemd/    assistantd, HUD, Parakeet STT user units
scripts/    install, Hyprland bind snippet, HUD launcher
docs/
```


Conversation and desktop automation
----------------------------------
After each answer you have ten seconds to ask another question without the wake word. Say **start conversation** for a continuous session, or **end conversation** to stop; sixty seconds of silence also ends it. Alt+Space still takes over, and F9 dictation remains available.

Say **start dictating** while your destination app is selected. **Pause dictation**, **resume dictation**, and **stop dictating** control the session. Focus changes pause insertion and retain a draft. Terminal dictation retains a draft; **insert draft** inserts it without pressing Enter.

Weather uses [Open-Meteo](https://open-meteo.com/en/docs) directly for timestamped current conditions and forecasts, using your location or an explicitly named city. Say **what is the weather** or **weather in Chicago**. US weather always uses Fahrenheit and miles per hour. Current conditions are weather-model estimates. Other current or unfamiliar factual questions use the configured search provider, without accounts or API keys. This installation uses the [AstroCorp DuckDuckGo MCP server](https://github.com/AstroCorp/MCP-DuckDuckGo), with bounded content extraction and cited results. Say **weather in Chicago** or **remember weather location Chicago**. Visible page requests use Google in a separate assistant Chromium profile. **Open that source** opens a cited link directly. If a site challenges the browser, say **take over browser**, solve it in the window, then **resume browser**. Automation does not bypass logins or CAPTCHAs.

Say **run the command … in the terminal**, **open a terminal**, or give a browser task such as **search Google for Rust documentation**. Generated commands and browser clicks/typing show exact arguments for approval: say **approve** or **deny**, or use Y/N in the HUD/TUI. Configured commands use `[terminal.commands]` argument arrays. Jobs run for 120 seconds by default (configurable up to 600), with output, cancellation, and exit status.

CLI: `conversation start|stop`, `dictation start|pause|resume|stop|insert`, `browser-takeover`, `browser-resume`, and `jobs status|follow|cancel ID`. TUI: Ctrl+O conversation, Ctrl+D dictation, Ctrl+P pause/resume, Ctrl+G/B browser takeover/resume. Session status, remaining time, retained drafts, tool progress, and sources are available through IPC and Activity. `doctor --json` checks browser assets and dictation dependencies.

Installation bundles Playwright 1.63.0 and its matching Chromium locally. Node 18+ and npm are required during installation; a stable Node wrapper is installed for systemd. The default desktop browser is unchanged.

Useful next additions: opt-in local file/content indexing with exclusions, account-connected email and calendar, reminders, reusable voice macros, and accessibility-driven control of native apps. These remain future work.


Location: weather and nearby questions use an approximate city from your public IP, cached in memory for one hour. Say **where am I**, **remember my location Chicago, Illinois**, **forget my location**, **disable location**, or **enable location**. CLI: `location show|refresh|set CITY|clear|on|off`. TUI Features: Location toggles automatic use; `l` edits the city override, then Ctrl+S applies it. Location is city-level rather than GPS, so a VPN or ISP can affect the estimate. Requests go to [ipapi.co](https://ipapi.co/api/), with [ipwho.is](https://ipwhois.io/documentation) as a fallback; no API key is required. Failed lookups ask for a city. Coordinates can be supplied explicitly in `[location]` if needed. The older weather-specific override remains supported.

Local abilities now run as managed MCP plugin processes. Press **0** in the TUI
for Plugins, or use `nixe-assistant plugins list`. Packages can be installed,
enabled, disabled and reloaded without reloading models. Notes and one-time
reminders persist in plugin-owned SQLite databases. See [Plugin API, migration
and authoring](docs/PLUGINS.md) and [the echo example](example-plugins/echo).

Desktop abilities now include named routines, project context, explicit
clipboard/screen OCR, a local calendar, optional file-content search, media
controls, battery/idle model management, and shared undo. See
[usage and permissions](docs/DESKTOP-PLUGINS.md). Indexing starts with no folders;
calendar account synchronization needs a provider connection.
