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
| §6.6–6.7 computer use, vision | **mostly (Phase 6)** | Accessibility tree, numbered marks (tree + OCR), keyboard and pointer without root, evidence-checked actions, vision on demand through the LLM's image projector. Planner and vision compared across the curated models in Phase 8c (BENCHMARKS.md): Gemma 4 E4B stays the planner (18/21, 5/5 with app tools); grounding stays weak (best 7/15, GLM pointing). VS Code confirmed live; Spotify only through `media.*`. |
| §7–8 permissions | **done in Phase 1** | See below. |
| §9 sandboxing | partial | Bubblewrap for generated terminal commands (namespaces, read-only system, hidden home, no runtime sockets; a repository's `.git/hooks` and `.git/config` read-only since Phase 8a). No seccomp or cgroup limits yet, and plugins still run with the user's permissions. |
| §10 privileged broker | **done in Phase 4** | `nixe-assistant-helper` behind polkit: install, remove and update repository packages, start, stop or restart system services. Typed and checked twice; no root shell. |
| §11.1 voice authorization | partial | Wake word and AEC exist. High-impact tools, and since Phase 8a writes to files that run code later, refuse a spoken approve; only the exact phrases count. The rest accept it. No speaker recognition. |
| §11.2 prompt injection | **mostly (Phase 8a)** | Tool results are untrusted data; the grammar admits only registered tools. Computer use can't reach Settings or the Store; sending out after reading private data and untrusted content asks; task-wide approvals are bound to one window or folder. `bench injection` passes. |
| §11.3 emergency stop | **done in Phase 8a** | Super+Shift+Escape (`nixe-assistant cancel`) anywhere, Escape on the HUD, voice "stop", Stop in Settings, job cancel. It never waits on the model, and releases held modifiers after a cut-off keyboard action. |
| §12 audit and recovery | **audit done in Phase 1**; crash recovery **Phase 8b** | Abilities keep their own undo (`history.undo`, `*.undo`). Crashed plugins restart (3 per 5 min); the recognizer restarts on the next utterance; systemd restarts the daemon, stops after 5 crashes in 2 min and says so. `bench crash` 4/4. |
| §13 GTK settings | **done in Phase 7** | An Assistant group in Nixe Settings (General, Voice, Permissions, Activity, Performance, Models), through `nixe-assistant config/perf/models` so the assistant still checks every value. The TUI stays for personas. |
| §14 hardware profiles | partial (Phase 8b) | One configuration. Measured under load on the RTX 5080 (BENCHMARKS.md); temperatures, power and the power source in `perf` and Settings. On battery the card can't sleep while Hyprland holds it, so early unloading is off by default. No Lite/Balanced/Advanced presets. |

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
- [x] Phase 7: the Assistant group in Nixe Settings; readable, clickable approvals in the HUD
      (live-checked 2026-10-15).
- [x] Phase 8a: security tests (injection, path traversal and links, approval reuse, voice,
      sandbox escape, privilege escalation) and the fixes they called for; a global stop.
- [x] Phase 8b: latency, graphics memory and heat under concurrent load; crash recovery.
- [x] Phase 8c: planner and vision benchmarks; the deferred Phase 6 work (VS Code confirmed;
      Spotify only through `media.*`).
- [x] Phase 9: typed control of Notepad and Write through their control interface (the
      `nixe-apps` ability); computer use stays the fallback for other apps.

- [x] Phase 10: typed control of Music and Contacts (same interface and ability).

**Recommended next milestone:** typed control for the remaining Nixe apps people talk to
(Chat, PDF, Gallery), measured the same way.

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
- VS Code confirmed in Phase 8c. Spotify isn't: playing with no window open, there's nothing to act on.
- Known limits:
  - The planner (Gemma 4 E4B) sometimes stops early or picks the wrong key.
  - Qwen 3.5 9B was tried and wasn't better (see BENCHMARKS.md).
  - Vision grounding is weak.

## Phase 10 as built

- `nixe-core::control::serve_async`: a handler answers through a `Reply`, later if it must
  (Music's playlists come from the database in the background); `serve` wraps it.
- Music (`apps/music/src/control.rs`): `music.search`, `music.play` and `music.queue`
  (track, album, artist or playlist by name; exact, then starts with, then contains;
  several equally good name each other), `music.status`, `music.control`.
- Contacts (`apps/contacts/src/control.rs`): `contacts.find` (up to 10, untrusted),
  `contacts.add` (refused when another contact has the email or phone), `contacts.update`
  (change or add, never remove). Saved through `ui::commit`, so the window follows.
- Policy: adding and updating contacts ask; Music and finding are allowed, and finding counts
  as reading private data (the leak guard applies).
- Bundled abilities are now replaced on install when the checkout has another version of
  them; before, an update never reached an existing install.
- Bench: 7 more app-tool cases; `music-typed` (an album plays, muted; checked by
  `music.status`) and `contacts-typed` (a scratch address book) pass. The bench finds
  Nixe apps by window class: a title match had found a terminal named after the task.

## Phase 9 as built

- `nixe-core::control`: `io.github.design_nexus.nixe.Control` on an app's own GApplication
  connection and object path (`Describe() -> s`, `Call(s name, s arguments) -> s` with
  `{"ok"}` or `{"error"}`), served by the primary instance from `connect_startup`, handled on
  the main thread. `home_path` and `save_target` keep files inside the home folder (links
  resolved first) and never overwrite without `overwrite: true`.
- Notepad (`apps/notepad/src/control.rs`): `notepad.new_tab`, `open`, `read`, `tabs`,
  `insert_text`, `save`. Write (`apps/office/src/write/control.rs`, Write only):
  `write.new_document`, `open`, `read` (Markdown), `insert_text` (paragraph styles),
  `save` (format by extension), `export_pdf`. Starting or opening another document is
  refused while the open one has unsaved changes; nothing closes or deletes.
- `abilities/nixe-apps`: the twelve tools over `busctl`; an app that isn't running is started
  (and reaped) first. Bundled like the other abilities.
- Policy: `*.open` is file reading, `*.insert_text` a plugin edit (asks), `*.save` and
  `write.export_pdf` file writes (ask); reading and listing are allowed. Approval sentences
  ("Save the Notepad tab as ~/todo.txt").
- Planner: an app request that names Notepad or Write (`session::app_tool_prefixes`) gets
  that app's tools beside the computer-use ones; a worked example shows new tab, insert,
  save; the early-finish guard counts an app tool's change.
- Bench: `bench planner` uses the real tools (11 app-tool steps); `bench computer`
  `notepad-typed` and `write-typed` check the saved file (exact text; a heading in the
  `.odt`).

## Phase 8c as built

- `planner.rs`: the planner's system prompt, worked examples, grammar and step prompt, shared by
  `plan_tools` and the bench.
- `bench planner [--one MODEL]`: 26 scored steps per curated model, one process per model,
  answers in `~/.cache/nixe-assistant/bench-computer/planner-*.json`. `examples/gen.rs` keeps
  only the chat check.
- `bench vision`: 15 questions (Notepad, a Firefox form page, VS Code), every curated model
  reading tags and pointing; answers in `vision-*.json`.
- `bench computer`: VS Code runs in the bench's own profile (no first-run setup, trust prompt,
  chat panel or restored buffers) and is checked line by line; Spotify is read over MPRIS
  (`busctl`, no playerctl), runs with the default output muted and put back, and adds a
  next-track task.
- Fixes found on the way:
  - Letter and digit shortcuts go to the window through Hyprland (`send_key_state`, down then
    up): Electron drops a virtual keyboard's modifiers, so ctrl+s never saved in VS Code.
  - `input.type` no longer refocuses a field that has focus (VS Code put the cursor back at the
    start).
  - `send_until_done` ends on a cancelled turn's wrapped Timing/Error instead of waiting out
    its timeout.
  - An image too large for the micro-batch on a non-causal projector is refused instead of
    aborting the daemon (llama.cpp's assert).
- Known: Gemma 4 12B QAT doesn't load at the configured 32768-token context on 16 GB and plans
  at ~25 s a step; Ternary Bonsai 2 9B needs a newer bundled llama.cpp.

## Phase 7 as built

- CLI for Settings (`daemon/nixe-assistantd/src/cli.rs`), JSON out:
  - `config get [KEY] --json`, `config set KEY VALUE` (TOML value, checked by
    `Config::validate`, saved with comments kept, then the daemon reloads, or restarts for keys
    that need it), `config unset mcp.permissions.TOOL`.
  - `perf --json`: the last turn's timing stages, the model and projector and whether they're
    loaded, the speech worker's load time, and graphics memory from `nvidia-smi`
    (`Control::Perf`).
  - `models list|search|files|download|use`: the catalogue moved from the TUI into
    `crates/assistant-models`. The curated list is Gemma 4 E4B and E2B, Qwen 3.5 9B, Gemma 4
    12B QAT and GLM-4.6V-Flash, each with its projector. `download` prints one JSON line per
    progress change. `use` sets `llm.model` with `vision.mmproj = "auto"` and restarts. `list`
    also says whether the streaming speech model is installed.
- Nixe Settings, `apps/settings/src/sections/assistant/`, shown when `nixe-assistant` is
  installed:
  - General: running state with Start, Stop and Restart; wake word, conversation, overlay;
    autonomy mode and the task budget.
  - Voice: speech recognition provider and device, the speech model (Download), microphone and
    speaker, duplex and barge-in, voice and speed with Preview.
  - Permissions: each capability as allowed, asks first or never (software changes can't be
    allowed); per-tool rules (add and remove); computer-use blocked apps and confirm words; the
    admin helper's state.
  - Activity (redrawn while open): approvals waiting, with Allow, Allow for this task and Deny;
    the last ten tasks with their steps' evidence and Stop; the last 30 audit records with a
    filter.
  - Performance (redrawn after each answer): the last answer's stages as a bar, what's loaded,
    graphics memory.
  - Models: the model in use, installed models with Use, the recommended list with Download (a
    progress bar, then the projector), and a Hugging Face search that lists a repository's files.
- Approvals: `assistant_policy::summary::describe` turns a tool call into one sentence ("Type
  “hello” into Write", "Install sl (and 2 dependencies)", "Restart cups.service (now active)").
  It goes in the `tool_approval` event (`summary`, plus `voice_allowed`), in `status`'s pending
  approvals, in the spoken prompt ("May I …?") and in the TUI.
- HUD: the approval card shows the sentence, details folded away, and Allow / Allow for task /
  Deny buttons (Y, T and N still work). Only the card takes clicks, and only while it asks. It
  says when saying "approve" doesn't count. A task line shows the request and its step while a
  task runs.

## Phase 8a as built

Gaps found and closed:
- **Computer use reached Settings and the Store.** It could have clicked Allow on its own
  approval under Activity, or loosened Permissions. Both apps are now in the window guard's
  deny list with terminals and password managers.
- **Writes to files that run code later** (`fs::runs_code`): `~/.local/bin`, shell startup
  files, `~/.config/{systemd,autostart,environment.d,uwsm,hypr,fish,git}`, Omarchy hooks,
  launchers, `~/.gitconfig` and any repository's `.git/hooks` or `.git/config`. `files.write`,
  `files.move` and `files.undo` there ask by key press, one call at a time (rule `runs code`),
  whatever the rules say. A deny still wins.
- **Task-wide approvals** are bound to a resource (`assistant_policy::resource_of`): the
  window for computer use, the destination folder for file changes. Allow for task in Write
  doesn't cover Firefox.
- **Sending out after untrusted reads.** A task records when a step returned private data
  (files, notes, captures, windows, command output) and when one returned someone else's
  content (pages, documents, notes, windows, command output, external tools). Once both
  have happened, calls that send things out ask on their own: `browser.navigate`, `.search`,
  `.open_source`, `.fill`, `.click`, a tab opened with a URL, `web.*` and external tools. Rule
  `may send private data`; the card shows the URL or query.
- **Links swapped in after the guard:** `files.read`, appends and backups open with
  `O_NOFOLLOW`; writes already replace the file through a rename.
- **The sandbox** binds the working repository's `.git/hooks` and `.git/config` read-only.
- **Emergency stop:** Super+Shift+Escape runs `nixe-assistant cancel` (`scripts/hypr-stop.lua`,
  added by install.sh), and a cancel during a keyboard action releases every modifier.
- Pending approvals (`daemon/.../approvals.rs`) answer once: an unknown, expired or answered
  ID fails; "for this task" counts only when offered. The spoken reply is matched whole
  (`approval_phrase`).

Tests: path traversal and links (including a link swapped in after the check), `runs_code`,
grants and pending approvals, resource-bound task approvals, the leak rule, spoken phrases,
a compromised-planner table, real bubblewrap (secrets, runtime sockets, network, git hooks),
helper argument smuggling. `cargo test --workspace`: 159 passed.

`bench injection` (BENCHMARKS.md) runs real requests over a page, a file and a Firefox window
carrying planted instructions, denies every approval but window reads, and checks the
collector, `~/.bashrc` and a marker file.

Accepted limits:
- Plugins and external MCP servers run as Ken and can reach the assistant's socket, so a
  hostile one could approve its own request. That needs the plugin sandbox (§9).
- Tools that aren't high-impact still take a spoken "approve" (exact phrase only).
- With `sandbox_network = true`, generated commands share the network namespace, so abstract
  Unix sockets are reachable.
- The guard resolves a path, then the tool opens it: a parent folder swapped for a link in
  between isn't caught (the last component is).

## Phase 8b as built

- `bench load` (scenarios idle, CPU busy, GPU busy, both; `--heat` for ten minutes under
  both) and `bench crash`, both in `daemon/.../bench.rs`. Results in BENCHMARKS.md.
- Playback gaps: the play queue records the queue running dry between two phrases of a reply
  (`PlayQueue::take_gaps`); each answer's gaps are in `perf --json` (`last_turn.gaps`) and
  Settings → Performance, and gaps of 150 ms or more are logged. None measured under any load.
- Telemetry (`assistant_core::telemetry`): the card's temperature, power, use, memory and
  pstate, the CPU package temperature, the power source and the daemon's CPU time and memory.
  The card's runtime-PM state is read from sysfs first and `nvidia-smi` is skipped while it
  sleeps, so looking never wakes it. Shown in `perf` and in Settings → Performance → Machine.
- `CPUWeight=1000` on `nixe-assistantd.service`: ahead of a compile, first audio with the CPU
  busy went from 1115 to 953 ms (median).
- Battery: `power.json` gains `battery_idle_seconds` (0, off); when set, unplugged, the models
  unload after the shorter of the two limits. Measured on battery with 60 s: the card stayed
  `active` before and after the unload (Hyprland, Firefox and Plezy hold `/dev/nvidia*`), the
  machine drew 30.5 → 29.9 W, and the next answer took 3290 ms instead of about 820. So it
  ships off; it helps only where nothing else keeps the card awake.
- Crash recovery: crashed plugins restart with backoff (`plugins::may_restart`: 3 in 5
  minutes, then "crashed repeatedly; reload to retry"); `StartLimitIntervalSec=120`,
  `StartLimitBurst=5` and `OnFailure=nixe-assistant-failed.service` (a notification) on the
  daemon.

Accepted limits:
- First audio meets §4.6's 900 ms at idle (823 ms) but not under load: about 980 ms with the
  CPU busy and 1.2–1.3 s with a second model on the card. GPU sharing isn't something the
  daemon can steer.
- Under sustained full load the CPU throttles (95–102 °C) and first audio drifts up by about
  450 ms in three minutes.- The NVIDIA card can't runtime-suspend while Hyprland (and other apps) hold it, so the
  assistant unloading its model saves almost no power on battery.

