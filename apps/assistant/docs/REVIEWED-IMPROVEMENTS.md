# Settings, tools and local interfaces

The TUI keeps a draft separate from saved configuration and the daemon's running
settings. Ctrl+S applies and acknowledges a reload. Ctrl+R applies and restarts.
The daemon reports every field that needs a restart; resident model, audio,
STT, HTTP and MCP connection settings remain truthful until then. Quit protects
unsaved drafts. Malformed configuration causes an error and cannot be replaced
by defaults. Saves retain comments and unknown fields and use atomic replacement.

Tabs 6–9 provide Diagnostics, Activity, Audio Calibration, and Apps & Commands.
Activity keeps 100 turns in memory, with transcripts, responses, stage timings
and exact tool arguments/results. `c` clears it. Calibration records five seconds
of quiet and five seconds of speech, measures RMS and (when loaded) Silero
probabilities, and offers suggestions. `a` copies them to the draft; Apply saves.
`t` previews the draft voice/speed through TTS only, with no saving, STT or LLM.

Apps & Commands edits individual JSON entries while preserving the surrounding
TOML document. Arguments are arrays, including arguments containing whitespace.
Phrase collisions are rejected. `p` previews routing without executing it.
Persona IDs remain stable when renamed; built-in alias deletions survive startup.

Downloads have one active task, cancellable connection/read waits, HTTP status
checks, GGUF metadata/tensor bounds validation and an atomic installation that
never replaces an existing model. Incomplete temporary files are cleaned up.
`/` searches Hugging Face; arrows focus repositories/files; Enter selects a
repository and then downloads a file. `u` downloads a direct HTTP(S) GGUF URL.
Split model files are unsupported and marked explicitly.

## MCP

Ordinary chat keeps its streaming path. Say “use tools to …” or use Ctrl+T to
toggle tool mode. The daemon initializes configured stdio servers, negotiates
supported versions (2025-11-25, 2025-06-18, 2025-03-26, 2024-11-05), discovers
paginated tools, and combines them with `files.read`, `git.status`, and
`system.status`. Initialization failures appear in Diagnostics.

Planning uses a JSON grammar: one registered tool with schema-valid arguments,
or finish. Clear browser/terminal requests enter tool mode automatically. The planner
sees only the tool groups a request names (notes, calendar, media, …), up to 24 tools,
or every tool when nothing matches; `null` for an optional argument counts as not
given. Each call has a 15 second limit. Results preserve `content`, `structuredContent`, and `isError`. Output
schemas are validated. Results are untrusted data, never instructions.

## Tasks

A tool request runs as a task with an ID, a state (planning, running, waiting for
approval, paused, then completed, partially completed, failed or cancelled) and a
budget: `[agent] max_calls` (default 4) and `max_active_secs` (default 180; approval
waits don't count). Every step is checked after it runs: errors fail; terminal jobs
are verified by exit status; abilities that return an undo record have confirmed
their change; desktop and app actions stay unverified for now. A task is completed
only when every change is verified, otherwise partially completed, and the spoken
answer says what couldn't be confirmed. A change that already succeeded with the
same arguments is not repeated within the task.

An approval can cover the rest of the task (`t` in the TUI, `T` on the HUD,
`approve ID --task`, or "approve for this task"): later calls to the same tool in
that task run without asking, for at most ten minutes. Generated commands, shutdown
and captures are always approved one call at a time. Browser takeover pauses the
task; "resume browser" continues it with its steps and remaining budget. Cancel
(`cancel`, Escape, `tasks cancel ID`) stops the task and its terminal jobs. A
restart marks tasks it interrupted as failed; nothing resumes on its own.

Tasks and steps are kept in `agent.db`: `nixe-assistant tasks`, `tasks show ID`.

External tools require confirmation by default. The TUI and HUD display exact
arguments; `y` allows once, `n` denies. Approvals expire after 60 seconds.
Cancellation and denial prevent dispatch. CLI equivalents are `approve ID` and
`deny ID`. A spoken "deny" always counts; a spoken "approve" does not for
generated terminal commands, shutdown or screen and clipboard captures, which
need the HUD, TUI or CLI, since a voice nearby or a video can say it.

Every call goes through one decision point (`assistant-policy`). An explicit deny,
per tool or in `[permissions]`, beats any allow. Generated terminal commands and
scripts always ask. `[agent] mode` tightens the rest: `autonomous` (default) runs
whatever is allowed, `assisted` asks before changes other than desktop, volume and
app launches, `manual` asks before any change. An allowed or approved call gets a
grant bound to the tool and a digest of its exact arguments, valid for one use
within 60 seconds; the registry dispatches nothing without one. Denials,
approvals and call outcomes go to `~/.local/state/nixe-assistant/agent.db` with
secret-looking values redacted; `nixe-assistant audit` lists them.

Permission overrides use the names displayed in Diagnostics:

```toml
[mcp]
enabled = true

[mcp.servers.example]
command = "my-mcp-server"
args = ["--stdio"]
# env = { SOME_SETTING = "value" }

[mcp.permissions]
"example.harmless_tool" = "allow"
"example.write_file" = "confirm"
"example.delete_file" = "deny"
```

Built-in file/git access canonicalizes paths (symlinks included) and restricts
them to HOME. Secret folders (`~/.ssh`, `~/.gnupg`, password stores, keyrings,
browser profiles) are off limits to every file tool, and the assistant's own
config, state and data are read-only to them. Reads are limited to 64 KiB and
500 lines. `files.write` (create, overwrite or append, up to 1 MiB),
`files.move` and `files.trash` keep a backup in
`~/.local/state/nixe-assistant/file-backups`, and `files.undo` (or "undo that")
puts it back; they ask unless `permissions.files_write = "allow"`.

Typed desktop and system tools: `windows.*`, `workspaces.*` and `monitors.list`
over Hyprland's socket; `services.*` for systemd user units (control asks unless
the unit is in `[system] user_units`; the assistant's own units are refused);
`network.status` and `network.wifi`; `audio.devices`, `audio.set_volume` and
`audio.set_default`; `system.power_profile`. Each change re-reads what it
changed and reports `verified`.

Commands the assistant writes (`terminal.run`, `terminal.shell`) run in a
bubblewrap sandbox: the system read-only, an empty home with only the working
folder writable and `[terminal] sandbox_read_only` readable, a private /tmp,
network per `sandbox_network`, and no `XDG_RUNTIME_DIR` (so no Wayland, D-Bus
or assistant socket). Without a project folder the job gets a scratch folder in
the cache, removed after a day. `sandbox: false` runs a command unsandboxed;
both ways still ask. Configured commands run as you. External MCP servers control their own
filesystem access: configure their allowed directories independently. Processes
are killed on teardown; disconnected requests fail promptly; cancelled requests
send MCP cancellation notifications. The client does not advertise sampling,
roots, elicitation, resources or tasks capabilities.

Protocol references: [tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools)
and [lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle).

## Local interfaces

First-party CLI, TUI, HUD and plugin controls use owner-only Unix IPC. Additive
`request` commands carry `request_id` and a tagged action. `result` events report
success/error with the same ID. `turn` events carry turn IDs and transcripts,
responses, timings or tool activity. `settings` publishes running settings,
restart requirements, and effective HUD enablement. `tool_approval` carries
arguments, approval ID and expiry. Existing commands/events remain supported.

HTTP binds only to a loopback IP. Every endpoint requires
`Authorization: Bearer TOKEN`. The randomly generated token is in
`$XDG_RUNTIME_DIR/nixe-assistant/http.token`, mode 0600 (runtime directory 0700).
HTTP action responses indicate queued work, not successful execution; use IPC
for acknowledged reloads. No browser-origin access is enabled.

CLI additions: `doctor [--json]`, `status --json`, `activity [clear]`, `audit [--limit N] [--json]`, `tasks [show ID|cancel ID]`,
`preview [voice speed]`, `calibrate`, `tool-mode on|off`, `approve ID`, `deny ID`.
`doctor` works when the daemon is stopped.

## Installation

`scripts/install.sh` builds and installs locally, backs up installed files and
configuration, migrates configuration additively, restarts affected services,
and verifies model health. Startup failure restores the backup. HUD assets live
in `~/.local/share/nixe-assistant/quickshell`, independent of the checkout.
The plugin keeps any existing assistant plugin identifier used by the bar.

CUDA is an optional build feature. The installer selects it when `nvcc` exists;
manual builds use `cargo build --release --features cuda -p nixe-assistantd`.
For this host's Blackwell GPU, put `CMAKE_CUDA_ARCHITECTURES=120a-real` in
`~/.config/nixe-assistant/build.env`; other hosts choose their own override.
Runtime library overrides go in `runtime.env`, for example
`LD_LIBRARY_PATH=/opt/cuda/lib64`. Existing user config is preserved.
