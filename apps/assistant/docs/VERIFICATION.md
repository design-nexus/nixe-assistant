# Verification — 2026-10-07

The reviewed fixes and the conversation/browser/terminal release are installed in the current desktop session. Existing configuration, persona files, desktop plugin identity, F9 dictation, and build/runtime overrides were preserved. Source changes remain uncommitted.

Validation completed:

- Workspace tests: 49 Rust tests passed. Browser tests: two tests passed, including a real Chromium fixture covering stable element references, detached references, HTTP failures, tab handling, and takeover boundaries.
- Rust formatting, whitespace checks, strict workspace Clippy, and CUDA release builds passed. Subsequent daemon changes received affected Clippy/build checks and focused runtime verification.
- Live daemon MCP navigation opened example.com in the separate visible Chromium profile. A generated `printf` job displayed exact arguments, waited for approval, returned the expected output, and exited with code zero.
- The synthetic Kokoro → Parakeet → local model → Kokoro voice loop passed. Three consecutive microphone-stream requests completed in conversation mode without a wake word. Two additional requests completed in follow-up mode without a wake word. Temporary PipeWire connections were restored.
- Terminal dictation retained a draft without typing or pressing Enter. TUI conversation, dictation pause/resume/stop, countdowns, and Activity scrolling were checked. The HUD was visually inspected and loaded without QML errors.
- Daemon, Parakeet, and HUD services are active. Health reports a loaded model, 23 registered tools, and no MCP errors. Installed binaries match the release artifacts.

DuckDuckGo returned a human-verification challenge during the live search check. Challenge detection and the honest failure response are implemented; successful factual retrieval after manual challenge completion remains dependent on the site. Automation does not solve CAPTCHAs.

Installation backup: `~/.local/state/nixe-assistant/backups/20261007-020421`. Intermediate updates also retained prior binaries and HUD assets within that backup directory. The installer restores the prior installation if startup health fails.

## Location update

On-demand approximate IP location, a manual city/coordinate override, voice controls, CLI controls, the TUI city editor, local search context, and the native `location.current` tool are installed. Automatic lookup uses HTTPS with bounded time/response size, an hour-long memory cache, and a fallback provider. Disabling location prevents lookup; saved city overrides need no network request.

- Affected core/tools/TUI/daemon tests passed (27 tests); the final provider normalization and configuration-preservation regression fixture passed with the 12 core tests.
- Strict workspace Clippy passed, followed by affected checks after the final changes. Formatting, whitespace, and CUDA release builds passed.
- The primary provider returned HTTP 429 during the live check. The fallback successfully returned approximate Calera, Alabama, United States. Repeated lookup reused the cache. Saved-city and disabled lookup controls passed; unrelated configuration values were unchanged.
- An explicit tool request dispatched `location.current`, returned its structured result without error, and streamed a spoken answer.
- “Where am I?” produced the expected approximate city response and TTS playback events. TUI location scrolling, city editing, unsaved-change protection, and discard were checked through a terminal session.
- Daemon, HUD, and Parakeet services are active; health reports a loaded model, 24 tools, and no MCP errors. All three installed binaries match the final release artifacts.

Location installation backup: `/home/ken/.local/state/nixe-assistant/backups/20261007-113236-location`. Configuration migration was additive; the original assets and config are retained for rollback. No precise device/GPS location is claimed.

## Current weather and US units fix

Weather previously used general search, which encountered a human-verification challenge. Weather now uses the Open-Meteo weather and geocoding APIs directly. The registry exposes `weather.lookup`; ordinary current-weather questions receive a deterministic reply from validated data, with source, units, location, and local timestamp. Forecast data covers seven days. Requests validate status, size, completeness, and freshness, and can be cancelled.

US locations enforce Fahrenheit and mph, including generated tool calls. The general system prompt also requires imperial units for US users. A state-lock lifetime issue during follow-up takeover was fixed; the state guard is released before cancellation.

- 27 affected core/tools/daemon tests passed; final core tests passed after the units and freshness changes. Strict affected Clippy, formatting, whitespace checks, and CUDA release builds passed.
- Two consecutive live requests succeeded: automatic Calera weather returned 79°F, feels like 80°F, and explicit Chicago weather returned 72°F, feels like 74°F. Both reported Open-Meteo data at 12:15 local time on 2026-10-07. Each returned seven days of structured forecast data. The second request completed during follow-up listening without stalling.
- The Celsius verification request was stopped following the user's imperial-units instruction. Final live verification used Fahrenheit only.
- Configuration was unchanged. Daemon, HUD, and Parakeet services are active; health reports 25 tools, a loaded model, and no MCP errors.

Installation backup: `/home/ken/.local/state/nixe-assistant/backups/20261007-122821-weather`.

## DuckDuckGo MCP integration

The AstroCorp server linked from the user's MCP Market page is installed at revision `0cd5c5b9e8a4994736f18ef816c601dddf5a8324`, with locked dependencies and a reviewable GPL-3.0-only local patch. Although curl received a challenge from the HTML endpoint, the actual Node stdio MCP server successfully retrieved results and page content. The daemon now routes factual questions to `duckduckgo.search`; weather remains on Open-Meteo and visible searches remain on Google. Only this external search tool received a new allow permission.

The local patch bounds HTTP time/body size and returned content, propagates cancellation signals, detects challenges, extracts main/article content without image URL noise, and returns structured citations plus publication/retrieval metadata. `scripts/install-search.sh` reproduces the installation and retains any prior search assets. `search-provider show|builtin|duckduckgo` manages provider selection; changes apply after restart.

- Affected core/tools/daemon tests passed (27 tests). The additional daemon regression passed (four daemon tests total): retrieved commands cannot trigger routing, while genuine user commands still route normally.
- Strict affected Clippy, formatting, shell syntax, whitespace checks, and CUDA release builds passed. Browser tests passed (two), including external-citation retention/opening and unsafe URL rejection. Small search fixtures passed for oversized responses, challenge handling, cancellation, and content extraction.
- The first full search check exposed retrieved source text entering desktop routing and switching to workspace 5. Routing now checks only the original user request; enriched source/tool input stays on the answer path. The repeated check retrieved three results, including official NASA Artemis II/III pages, and streamed a spoken answer with no desktop command dispatch.
- The active daemon has 27 tools, a loaded model, no MCP errors, and `duckduckgo.search` as its running provider. Binaries, browser worker, and the installed upstream patch match their reviewed artifacts. Unrelated configuration values were preserved.

Final runtime installation backup: `/home/ken/.local/state/nixe-assistant/backups/20261007-124349-search`. The preceding search deployment was also retained under `20261007-123959-search`.
