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
| §4 full duplex | **done in Phase 5** | Streaming STT (Nemotron worker) with partial words in every listening mode; replies listened to while they play (pause at speech onset, then echo / backchannel / noise / stop / redirect); per-reply generation ids; the context keeps what was heard of a cut-off reply. AEC is PipeWire's webrtc module, which is AEC3. |
| §4.5 independent states | **done in Phase 5** | `Activity` (mic, recognition, reasoning, synthesis, playback, agent) in an `activity` event and `status --json`; the single `state` event stays for the HUD and TUI until Phase 7. |
| §4.3 models | partly measured | Nemotron streaming (STT, Phase 5) and Parakeet (fallback) measured; Kokoro and Gemma in use. Qwen3-TTS, PersonaPlex and Moshi untested (Phase 8). |
| §5 agent engine | **done in Phase 2** (core) | Tasks with IDs, states, budgets, per-step verification and resume after browser takeover. Planning is still one call at a time; there is no up-front plan or checkpoint/rollback. |
| §6.1 desktop | **done in Phase 3** | Typed window, workspace and monitor tools over Hyprland's socket; focus, move, close and switch re-read the state to verify. Phrase commands still use `hyprctl eval`. |
| §6.2 system | **mostly (Phase 3)** | systemd user units (list, status, control with an allowlist), NetworkManager status and Wi-Fi, PipeWire devices and volume, power profiles. Bluetooth beyond toggling and UPower details are not typed; D-Bus is still reached through commands. |
| §6.3 files | **done in Phase 3** | One guard for every file tool (secret folders off limits, the assistant's own folders read-only); list, search, write, move, trash with backups and `files.undo`. |
| §6.4 terminal | **mostly (Phase 3)** | Generated commands run in a bubblewrap workspace sandbox and still ask; configured commands run as the user. Admin work goes through the typed helper (Phase 4), never a root shell. |
| §6.5 browser | mostly | Playwright worker with takeover. |
| §6.6–6.7 computer use, vision | **mostly (Phase 6)** | Accessibility tree, numbered marks (tree + OCR), keyboard and pointer without root, evidence-checked actions, vision on demand through the LLM's image projector. The small planner model is the weak link; vision grounding is weak (Gemma 0/5, GLM-4.6V-Flash 3/5 when pointing). |
| §7–8 permissions | **done in Phase 1** | See below. |
| §9 sandboxing | partial | Bubblewrap for generated terminal commands (namespaces, read-only system, hidden home, no runtime sockets). No seccomp or cgroup limits yet, and plugins still run with the user's permissions. |
| §10 privileged broker | **done in Phase 4** | `nixe-assistant-helper` behind polkit: install, remove and update repository packages, start, stop or restart system services. Typed and checked twice; no root shell. |
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
- [x] Phase 2: tasks (ID, state, budgets, cancellation) in `agent.db`; task-wide
      approvals; a verification verdict per step; browser-paused tasks resume with
      unexpired approvals only; restarts mark live tasks failed.
- [x] Phase 3: typed tools for windows (Hyprland socket), systemd user units, NetworkManager,
      PipeWire and power profiles; guarded file writes with backups and trash; bubblewrap for
      generated commands; first-party manifests refresh on install.
- [x] Phase 4: privileged broker: a pkexec helper with one polkit action per verb
      (packages, system services), checked on both sides, key-press approval plus password,
      root audit log.
- [x] Phase 5: streaming STT and generation IDs; independent states; interruption handling
      with context repair; echo-rejection benchmark under load.
- [x] Phase 6: accessibility tree, marks, keyboard and pointer, evidence-checked actions, vision
      on demand (see "Phase 6 as built" for what does and doesn't work yet).
- [ ] Phase 7: "Agent & Permissions" in Nixe Settings; approvals in the HUD for high-impact tools.
- [ ] Phase 8: security tests (injection, path traversal, approval reuse), latency under load.

**Recommended next milestone:** Phase 7: "Agent & Permissions" in Nixe Settings and approvals
in the HUD; then Phase 8, where the planner model and vision grounding get a proper benchmark.

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
- Not yet in Phase 1: approval scopes beyond one call (Phase 2 added task scope; there is
  no session scope). The `sandbox` decision still asks: since Phase 3, generated commands
  run in bubblewrap and are approved each time, by Ken's choice.

## Phase 4 as built

- `crates/assistant-privileged`: `Action` (install, remove, update, service) with
  `parse`/`to_args`, the package and unit rules, protected packages, critical units, and
  the root-only extra deny lists in `/etc/nixe-assistant/admin.toml` (`deny_packages`,
  `deny_units`; honoured only when root owns the file and nobody else can write it).
- `nixe-assistant-helper`, installed root-owned at `/usr/local/lib/nixe-assistant/` by
  `sudo nixe-assistant install-helper` (the installer offers it when the helper or policy is
  missing or older). It refuses unless run as root by pkexec (`PKEXEC_UID`) from the plugin
  worker or daemon of that same user, parses its arguments again, runs pacman or systemctl
  by absolute path with a cleared environment, and appends one JSON line per request to
  `/var/log/nixe-assistant/privileged.log` (`nixe-assistant admin-log`).
- Polkit: one action per verb through `exec.argv1`. Installs and updates ask for the
  password every time (`auth_admin`); removes and service actions keep it for polkit's few
  minutes (`auth_admin_keep`).
- Packages come from the configured repositories only (`-S --needed`, never `-Sy`; no AUR,
  URLs or files). Removal is `-Rs` and refused when it would take a protected package (the
  base system, kernel and boot chain, systemd, D-Bus, the desktop) with it. Updates are
  `-Syu`. Services are `.service` units only, never D-Bus, systemd's own, logins, the
  display manager, polkit or the assistant's.
- Tools: `admin.install`, `admin.remove`, `admin.update`, `admin.service` (capability
  `Privileged`: always asks, needs a key press, no task-wide approval; `permissions.admin`
  can only make it deny) and the looking tools `packages.search`, `packages.info`,
  `services.system_status`. Before asking, the daemon runs the unprivileged dry run
  (`pacman -Sp`/`-Rsp`, `checkupdates`, the unit's state) and shows it with the approval;
  a missing or protected package fails there, before anyone is asked.
- Expiry: the assistant's grant (bound to the arguments, 60 s, once) plus polkit's own.
  Cancelling during the password prompt stops pkexec; once the helper runs as root it
  finishes, and the reply says to check `admin-log`.
- Not in Phase 4: "apply approved configuration" (staged `/etc` edits) was left out by
  Ken's choice; the power profile stays unprivileged (`powerprofilesctl`).

## Phase 5 as built

- `crates/assistant-asr`: `nixe-assistant-asr`, a warm sherpa-onnx recognizer in its own
  process (its ONNX Runtime never meets the one Kokoro and Silero load). Frames in on stdin,
  JSON partials and finals out (`assistant_core::asr`). It pads 0.25 s before and 1 s after an
  utterance. `nixe-assistant models fetch asr` installs the model; without it, speech falls
  back to Voxtype. `[stt] provider = "streaming" | "voxtype"`, `threads = 2`, `device = "cpu"`.
- `assistant_agent::asr::Stt` wraps both recognizers with Voxtype's method names plus `tap`:
  PTT, wake, follow-up and conversation listening feed audio as it is captured, so the final
  arrives ~140 ms after speech ends, and partial words go out as `partial_transcript`.
  Listening modes where the assistant is silent use the raw microphone; while it speaks, the
  echo-cancelled stream.
- Playback (`assistant-audio/src/playq.rs`): phrases are tagged with a reply generation and
  index; one reply's audio can be dropped without touching newer audio (and phrases still on
  the way are dropped on arrival); a ramped gain for ducking, a pause that loses nothing, and
  a record of how far playback got.
- Duplex (`daemon/nixe-assistantd/src/duplex.rs`): while a reply plays, Silero on the
  echo-cancelled stream (gate just above the measured echo residual, `[duplex] min_rms`) pauses
  the reply at speech onset (`on_speech = "pause"`, or `"duck"`), streams the words, and
  decides: echo / backchannel / noise carry on from the same spot; a stop phrase ends the reply
  and opens follow-up listening; anything else ends it and becomes the next request (a short
  start like "actually…" waits 1.2 s for the rest). Rule-based, no LLM call. The history keeps
  the reply as heard, cut back to a whole word, with "— [interrupted]". Interruptions never
  approve anything. Each decision is an `interruption` event.
- Changes from the plan, with Ken: pause at onset instead of ducking (short words said over the
  voice didn't survive echo cancelling); a missed quick "stop" is left alone rather than worked
  around, to keep backchannels smooth; no second recognizer. `nixe-assistant bench duplex`
  measures false interruptions and live trials (BENCHMARKS.md).

## Phase 6 as built

- `crates/assistant-desktop`:
  - `a11y`: AT-SPI over the `atspi` crate. A window's tree is found by the window's pid; ids are
    e1…; elements can be pressed, focused or have their text set directly. Whether an element
    can be operated comes from its role, because GTK labels expose clipboard actions too.
    `enable()` turns on the session's accessibility switch, so Firefox, Chromium and Electron
    expose their trees.
  - `screen`: window captures with grim, mapping image pixels to layout coordinates (the monitor
    is at 1.6).
  - `ocr`: tesseract TSV, grouped into lines.
  - `marks`: one numbered list from the tree and OCR, drawn on the screenshot for vision.
  - `input`: Hyprland's `hl.dsp.cursor.move` positions the cursor, a Wayland virtual pointer
    clicks and scrolls, and `wtype` types. No root is needed.
  - `guard`: password managers, terminals, polkit and lock screens are never touched.
  - `verify`: success needs evidence. For typing, the field's text or OCR of the window. For a
    press, click or key, a changed title, a changed set of elements or element states, or a new
    or closed window. Screen-pixel differences don't count.
- Tools: `ui.tree`, `ui.find`, `ui.act` (by id, or by name with window), `screen.marks`,
  `screen.look`, `input.click`, `input.type` (finds the window's text field, avoids a browser's
  address bar, types into apps that draw their own editor and confirms by OCR), `input.key` and
  `input.scroll`. They run in the daemon (shared ids, the model), as daemon-local registry
  tools.
- Policy: `Capability::ComputerUse` asks by default; approval needs a key press, and one approval
  may cover a task. A click on anything labelled with `[computer] confirm_words` (send, delete,
  buy…) returns `needs_confirmation`, and `confirmed: true` is always asked separately.
- Planner support (small models need it):
  - a `Computer` request kind ("in X, type/press/open…")
  - worked examples in the planner prompt
  - the window filled in from the request when omitted
  - no "finish" before a confirmed action
  - actions never fall back to the focused window
- The HUD lets go of the keyboard while the assistant types or clicks (`computer_input` event).
  Before that, its own keystrokes landed in the HUD.
- Vision: `LlamaCppBackend::look` loads the mmproj next to the model on first use and unloads it
  after `[vision] idle_secs`; `<think>` blocks are stripped.
- Confirmed live through the planner, each checked against the app:
  - Write: typing into a new document
  - Notepad: a new tab
  - a Firefox page: search box and Enter
- Not yet confirmed: VS Code, Spotify.
- Known limits:
  - The planner (Gemma 4 E4B) sometimes stops early or picks the wrong key.
  - Qwen 3.5 9B was tried and wasn't better (see BENCHMARKS.md).
  - Vision grounding is weak.
  - `bench computer` still needs work. Its Spotify task only runs if Spotify is already open.
