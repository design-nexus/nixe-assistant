# Assistant 2.0 — repository audit (2026-10-13)

This compares `apps/assistant` with the "Omarchy Voice Assistant 2.0" plan (the PLAN.md
Ken drafted; sections are cited as §N). It records what already exists, what can be
reused, what is missing and what is risky, and gives a checklist by phase. Phase 1
(the permission foundation) was done alongside this audit; see the end.

## What exists

| Area | Where | State |
|---|---|---|
| Capture and playback | `crates/assistant-audio/src/engine.rs` | PipeWire 0.9, 16 kHz capture, 24 kHz playback, queue flush on cancel (7 µs measured) |
| Echo cancellation | `engine.rs`, `docs/plan.md` Phase 2 | PipeWire `module-echo-cancel` (webrtc) with a wire loop feeding the speaker monitor as reference; used for barge-in only |
| VAD | `crates/assistant-audio/src/{silero,vad}.rs` | Silero v5 ONNX on CPU, energy fallback |
| Wake | `crates/assistant-agent/src/wake.rs` | STT-as-KWS: transcript prefix and name match; no wake model |
| STT | `crates/assistant-agent/src/stt.rs` | warm Parakeet through an isolated Voxtype daemon; whole-utterance, no partials |
| LLM | `crates/assistant-inference/src/lib.rs` | in-process llama.cpp CUDA, prefix KV cache, flash attention, JSON grammar for tool planning |
| TTS | `crates/assistant-agent/src/tts.rs`, `chunker.rs` | Kokoro ONNX on CPU (no sm_120 kernels in ORT), phrase chunking that overlaps LLM tokens |
| Turn state | `crates/assistant-core/src/state.rs` | one sequential machine: Idle → Listening → Transcribing → Thinking → Speaking (+ Executing, Interrupted, Error) |
| Barge-in | `daemon/nixe-assistantd/src/app.rs` | speech over TTS (after a grace window) or Alt+Space cancels playback |
| Sessions | `daemon/nixe-assistantd/src/session.rs` | follow-up listening, conversation mode, dictation into a tracked window |
| Desktop routing | `crates/assistant-agent/src/router.rs`, `config/commands.toml`, `config/apps.toml` | phrase table to `hyprctl`/`omarchy` argument arrays; no LLM |
| Tools | `crates/assistant-tools/src/registry.rs` | registry over plugins; schema validation; 12-call planning loop, 15 s per call, 180 s active budget |
| Plugins | `crates/assistant-tools/src/plugins.rs`, `crates/assistant-plugin-worker`, `abilities/*` | one MCP stdio process per plugin; first-party worker for terminal/weather/files/apps/desktop; Python abilities (notes, calendar, routines, power, media, …) |
| Terminal | `crates/assistant-tools/src/jobs.rs` | argument arrays, cwd inside HOME, bounded output, process-group kill on cancel/timeout |
| Browser | `browser/worker.mjs` | pinned Playwright MCP worker; fixed operations and element references, no arbitrary JS; separate profiles for background and visible tasks |
| External MCP | `crates/assistant-tools/src/mcp.rs` | stdio client, protocol negotiation, cancellation notices |
| Approvals | `daemon/nixe-assistantd/src/interaction.rs` | 60 s approvals by voice, TUI, HUD or CLI, showing exact arguments |
| Activity | `daemon/nixe-assistantd/src/app.rs` | last 100 turns in memory, raw tool arguments and results |
| Interfaces | `crates/assistant-ipc`, `crates/assistant-tui`, `quickshell/`, `daemon/.../cli.rs` | owner-only Unix socket, loopback HTTP with a bearer token, ratatui TUI, Quickshell HUD, CLI |
| Install | `scripts/install.sh`, `systemd/` | user services, backups, CUDA build switch |

Baseline (docs/BENCHMARKS.md, 2026-10-06, RTX 5080): speech end → first audio **788 ms**
with warm Parakeet, Gemma 4 E4B Q4 and Kokoro on CPU; ~3.8 GiB VRAM at idle. That already
meets §4.6's 900 ms target for a short reply; it was measured on its own, not under the
concurrent load §4.6 asks for. Tests: 62 passed before this work (73 after) (`cargo test --workspace`).

## Compared with the plan

| Plan | State | Notes |
|---|---|---|
| §4 full duplex | partial | Concurrent capture during playback for barge-in exists; STT is not streaming, there are no partial transcripts, and responses are not cancelled by generation ID. The PipeWire AEC module stands in for in-process AEC3. |
| §4.5 independent states | missing | One sequential state machine. Splitting it touches the HUD protocol (`state` events). |
| §4.3 models | different | Parakeet, Kokoro and Gemma are in use and measured. Nemotron, Qwen3-TTS, PersonaPlex and Moshi are untested candidates. |
| §5 agent engine | partial | A planning loop with budgets and cancellation exists per turn; there are no persistent tasks, IDs, checkpoints, verification step or resumption. |
| §6.1 desktop | mostly | Hyprland control goes through `hyprctl` argument arrays, not the IPC socket. Listing windows is missing as a typed tool. |
| §6.2 system | partial | volume, Bluetooth toggle, power profiles and media exist; NetworkManager, UPower and systemd unit control are missing; D-Bus is used through commands, not a client. |
| §6.3 files | partial | read-only inside HOME; no writes, moves, trash or backups. |
| §6.4 terminal | partial | managed jobs with approval for generated commands; no read-only or workspace sandbox. |
| §6.5 browser | mostly | Playwright worker with takeover. |
| §6.6–6.7 computer use, vision | missing | Screenshots and OCR exist as desktop actions only. |
| §7–8 permissions | **done in Phase 1** | See below. |
| §9 sandboxing | missing | No bubblewrap, seccomp or cgroup limits. Plugins run with the user's permissions (the CLI already says so). |
| §10 privileged broker | missing | Nothing runs as root. |
| §11.1 voice authorization | partial | Wake word and AEC exist. High-impact tools refuse spoken approval; the rest accept it. |
| §11.2 prompt injection | partial | Tool results are passed as untrusted data, and the planner grammar admits only registered tools. |
| §11.3 emergency stop | partial | `cancel`, Escape on the HUD, voice "stop" and job cancel exist; there is no global shortcut outside the HUD, and no synthetic input to release. |
| §12 audit and recovery | **audit done in Phase 1**; recovery missing | Abilities keep their own undo (`history.undo`, `*.undo`). |
| §13 GTK settings | different | Settings live in the TUI. Nixe Settings (GTK) is the natural home for an "Agent & Permissions" page; it reads the same `config.toml`. |
| §14 hardware profiles | missing | One configuration. |

## Reusable as is

The audio engine and cancel path, the Silero VAD, the STT boundary, the in-process LLM with
its grammar, the chunker, the plugin protocol and workers, the job runner, the browser
worker, the MCP client, the approval flow (voice, HUD, TUI, CLI) and the IPC. The plan's
crate layout (§15) is not worth adopting wholesale; new work slots into the existing crates,
plus `assistant-policy`.

## Risks

- **Voice approvals.** A spoken "approve" from a video or another person counted. Since
  Phase 1, generated commands, shutdown and captures need a key press; other tools still
  take a spoken approval (§11.1).
- **Plugin trust.** Plugins share the daemon's uid. The worker's grant check (Phase 1) stops
  accidental unchecked calls from the host, not a hostile process with the same uid; that
  needs the sandbox (§9).
- **Wayland.** Synthetic input and screen capture depend on compositor and portal support;
  computer use (§6.6) will cover fewer apps than X11 automation would.
- **GPU.** ORT ships no sm_120 kernels, so ONNX models (Kokoro, a vision model) run on CPU
  unless built from source. NVML on this host currently reports a driver/library mismatch,
  so `nvidia-smi` can't report VRAM until a reboot. STT, LLM, TTS and an on-demand vision
  model must share 16 GB.
- **Latency under load.** The 788 ms figure is a single turn. Streaming STT and concurrent
  agent work will compete for the same CPU threads Kokoro uses.
- **State machine split.** Independent states change the IPC events the HUD and TUI read.

## Checklist

- [x] Phase 0: this audit; baseline tests green (62).
- [x] Phase 1: typed decisions and capabilities, decision point, grants at the enforcement
      point, deny overrides, autonomy modes, audit journal, tests.
- [ ] Phase 2: tasks (ID, state, budgets, cancellation token) in the same `agent.db`; a
      verification step after state-changing tools; resumable only with a valid grant.
- [ ] Phase 3: typed tools for windows (Hyprland socket), systemd user units, NetworkManager,
      UPower; file writes with backups and trash; bubblewrap for workspace commands.
- [ ] Phase 4: privileged broker with polkit and typed actions.
- [ ] Phase 5: streaming STT and generation IDs; independent states; interruption handling
      with context repair; echo-rejection benchmark under load.
- [ ] Phase 6: accessibility tree and screenshot grounding; vision model on demand.
- [ ] Phase 7: "Agent & Permissions" in Nixe Settings; approvals in the HUD for high-impact tools.
- [ ] Phase 8: security tests (injection, path traversal, approval reuse), latency under load.

**Recommended next milestone:** Phase 2's task journal on `agent.db`, because tasks give
grants a scope beyond one call (approve once for a task) and give verification and
recovery somewhere to record results.

## Phase 1 as built

- `crates/assistant-policy`: `Decision` (allow, sandbox, ask, deny; config spellings
  `allow`/`confirm`/`ask`/`deny`), `Capability` (one table from tool names), `AutonomyMode`
  (`[agent] mode`), `decide` (the decision point), `Grant`, and `store` (SQLite journal).
- Order of `decide`: unavailable or disabled → deny; any explicit deny (per tool or per
  capability) wins; then the per-tool rule, untrusted plugins asking, the capability rule,
  the default; the mode may only tighten; generated terminal commands never go below ask.
- `Registry::call` takes a `&Grant` instead of `approved: bool`. A grant is bound to the
  tool and a SHA-256 of the canonical arguments, expires after 60 s, and works once. Only a
  verdict makes one: `Grant::allowed` for allow, `Grant::approved` after a person approves.
  `call` also re-decides, so a reload that tightens a rule stops calls already granted.
- The plugin manager forwards a checked grant to the worker as request `_meta`; the
  first-party worker refuses calls without a matching one, including its desktop and clock
  paths.
- `agent.db` in `~/.local/state/nixe-assistant` (0600, directory 0700) records denials,
  approvals and call outcomes with secret-looking keys and long values redacted.
  `nixe-assistant audit [--limit N] [--json]` reads it without the daemon.
- Behaviour changes: `files_read` now governs `files.read` and `git.status`, and
  `launch_app` governs `agent.handoff` (both were always allowed before, whatever the
  config said); a user `allow` on `terminal.run` or `terminal.shell` still asks; a capability
  deny (for example `volume = "deny"`) beats a per-tool allow. Defaults are unchanged.
- Not yet: approval scopes beyond one call (task, session), and the `sandbox` decision,
  which asks until Phase 3 adds bubblewrap.
