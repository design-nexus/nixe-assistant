# Local assistant abilities (API version 1)

The daemon keeps microphone capture, models, streaming, speech, conversation,
permission decisions and cancellation. Abilities run in independent MCP stdio
processes. Updating `nixe-assistant-plugin-worker` does not require rebuilding
the daemon or TUI. Processes are started only for enabled packages; no crash
restart loop is used. Use `plugins reload ID` to retry.

**Each plugin runs in an operating-system sandbox that its manifest declares**
(see "Sandbox" below). A plugin that declares nothing gets no network and none of
your home folder. Still review executable code and dependencies before installing:
what a plugin declares, it can use. The installer does not execute installation hooks. Native dynamic
libraries are not a plugin interface; scripts and compiled executables are.

Packages live in `$XDG_DATA_HOME/nixe-assistant/abilities/ID` (normally
`~/.local/share`). Settings, source metadata, migration trust and saved enablement
live separately in `$XDG_CONFIG_HOME/nixe-assistant/plugins.json`. Persistent
plugin files live in `plugin-data/ID` under the assistant's data directory.
Disable and remove retain these files. Back up SQLite databases and Markdown
exports as personal data. Notes are separate from conversation history.

## Commands and TUI

```
nixe-assistant plugins list --json
nixe-assistant plugins inspect notes --json
nixe-assistant plugins install ./example-plugins/echo --yes
nixe-assistant plugins enable example.echo
nixe-assistant tool example.echo '{"text":"hello"}'
nixe-assistant plugins reload example.echo
nixe-assistant plugins disable example.echo
nixe-assistant plugins update ./updated-package --yes
nixe-assistant plugins remove example.echo
```

Inspection is JSON and works offline. Mutating lifecycle controls are acknowledged
through owner-only IPC. CLI tool calls go through the same permission checks as
voice and LLM planning. Third-party tools default to confirmation. Use the TUI or
`approve ID` / `deny ID` for pending calls. Approvals expire after 60 seconds and
are bound to the plugin process; disabling or reloading invalidates them.

In the TUI, press **0** for Plugins. Space toggles draft enablement, **e** edits
settings as JSON, **Ctrl+S** applies the draft, and **r** reloads/retries the
selected process. Quitting without Apply discards the draft. Inspection shows
saved and running enablement, version, tools, dependencies, settings and errors.
Required dependents must be disabled before their dependency, or included in the
same TUI draft. Optional dependencies may remain disabled.

Install accepts a folder, `.tar`, `.tar.gz`, or `.tgz` with `plugin.toml` at the
root. Symlinks, hardlinks, special files and paths escaping the root are rejected.
Archives are limited to 128 MiB of declared uncompressed file sizes. Git sources
require `--revision` with an exact 40-character commit hash. Install prints source,
startup command, tools and dependencies for review. `--yes` means the user has
reviewed the package. New third-party packages start disabled. Updates cannot
change declared tools, providers or required dependencies silently; use a new ID
for a reviewed permission surface change. Failed live update restores the prior
package. Failed reload keeps the working process and reports the new error.

## Package manifest

```toml
manifest_version = 1
id = "example.echo"
version = "1.0.0"
description = "Echo text"
executable = "python3"
args = ["./main.py"]
tools = ["example.echo"]
dependencies = []
optional_dependencies = []
providers = []

[settings_schema]
type = "object"
additionalProperties = false

[sandbox]
network = true
```

`executable` and `args` are argument arrays, never a shell string. Arguments
beginning `./` resolve within the installed package. Do not depend on the daemon's
working directory. The host provides `ASSISTANT_PLUGIN_ID`,
`ASSISTANT_PLUGIN_DATA`, `ASSISTANT_PLUGIN_SETTINGS` (JSON) and
`ASSISTANT_CONFIG` (compatibility settings for bundled workers).

### Sandbox

Every plugin runs in bubblewrap, under a seccomp filter, in its own systemd scope
(`nixe-assistant-plugin-ID-…`) with memory, process and CPU limits. `[sandbox]`
declares what it may reach; without one it gets the strictest sandbox. Inside:

- the system (`/usr`, `/etc`, `/opt`, `/sys`) read-only, its own `/proc`, `/dev` and
  `/tmp`;
- an empty home folder holding only its package (read-only) and its data folder
  (`ASSISTANT_PLUGIN_DATA`, writable), plus the folder its program lives in when that is
  under home (a bare command name is looked up first, and a one-line `exec` wrapper is
  followed to the program it runs);
- no network, no desktop sockets and a cleared environment (the variables above, `PATH`,
  `HOME`, `LANG`).

| Key | Grants |
|---|---|
| `network = true` | The network and DNS. |
| `home = "read"` / `"write"` | Your home folder. Secret folders (`.ssh`, `.gnupg`, keyrings, browser profiles) are always hidden; with `write`, the assistant's own folders stay read-only. |
| `read = ["~/Music"]`, `write = [...]` | Those folders or files only. Never secret folders; never the assistant's own folders for writing. |
| `sockets = [...]` | `wayland`, `pipewire`, `hyprland` (its control socket can start programs), `system-bus`, `session-bus` (filtered, below), `systemd` (the user service manager; needs `host_processes`). |
| `talk = ["org.mpris.MediaPlayer2.*"]` | With `session-bus`: the only names it may call, through `xdg-dbus-proxy`. A trailing `*` covers a prefix. |
| `devices = ["gpu"]` | NVIDIA and DRI device nodes. |
| `host_processes = true` | The computer's processes in `/proc`, not only its own. |
| `memory_mb`, `tasks`, `cpu_percent` | Limits (defaults 1024 MB, 256 tasks, 200%). |
| `unconfined = "reason"` | No bubblewrap or seccomp (limits still apply; defaults 8192 MB and 4096 tasks, no CPU limit). Only for plugins bundled with the assistant. |

The seccomp filter refuses calls a plugin has no use for: debugging other processes,
kernel modules and keyrings, BPF, perf, mounts and new namespaces, clock changes, and
typing into the terminal (TIOCSTI). `nixe-assistant plugins` shows each manifest's
sandbox. Configured MCP servers get `network = true` unless `[mcp.servers.ID.sandbox]`
says otherwise.

Bundled plugins that start programs on the desktop are unconfined: apps, desktop
commands, terminal, coding agents, admin (pkexec), the Nixe apps and the browser.
Generated terminal commands still run in their own sandbox, which now also has the
seccomp filter and limits (8 GB, 2048 tasks).

Tools must be declared in the manifest and discovered using MCP `tools/list`.
Input schemas must be valid JSON Schema objects. Results use MCP `content`;
`outputSchema` requires matching `structuredContent` on successful calls. The
host validates schemas, arguments and structured output. Frames are bounded to
1 MiB; requests have a 15-second deadline and propagate MCP cancellation. Plugins
must cooperatively cancel work and clean up their jobs. The host kills disabled
process groups. Keep protocol traffic on stdout; diagnostics belong on stderr.

Voice routes use exact phrases (`template = false`) or anchored regular expressions
with named captures (`template = true`). Regex patterns must start with `^` and
end with `$`; use `^(?i:...)$` for case-insensitive matching. Routes declare a tool
and pass `request` containing only the original user request plus named captures.
Schemas for routed tools must permit these fields. Retrieved content never enters
voice routing. Duplicate IDs, tool names, route patterns and provider roles are
rejected at activation. Different overlapping regexes should be avoided; exact
pattern collisions are detected at activation; requests matching different overlapping patterns are rejected before dispatch. General regex intersection is not inferred ahead of time.

The bundled worker retains existing configured desktop phrases, app and command
argument arrays, fuzzy phrase compatibility and coding-agent phrases. Clock prompt context is also gated by the Clock plugin
enabled state and permission. Weather and
factual lookup are automatic; general plans still require explicit tool mode and
have at most four calls. Visible Google requests use the browser. Implicit weather
resolves through the enabled Location provider; explicit-city weather works with
Location disabled. US weather uses Fahrenheit/mph.

## Python SDK and events

Copy `plugin-sdk/assistant_plugin.py` into a package; see
`example-plugins/echo`. Python's standard library is sufficient for this example,
Notes and Reminders. `Plugin` handles initialization, tool discovery, responses,
cancellation tokens and protocol errors. Handlers receive `(name, args,
cancelled)`; check the threading event before irreversible work and during long
loops. The host's approvals are permission decisions, not handler arguments.

Assistant extension v1 advertises `experimental.assistant.version = 1`.
Notifications use `notifications/assistant/event` with
`{version:1,id,title,text,speech}`. Only host-approved bundled/migrated plugins'
events are delivered currently; third-party events are ignored. The host may
invoke `assistant/event_ack` with `{version:1,id}` after successful desktop
notification delivery. Persist pending deliveries until acknowledged; retry
unacknowledged events with the same ID. Plugins do not invoke core speech. The
host speaks only when idle, in wake mode, outside approvals, and not muted or in
DND. Startup overdue summaries have no speech. `assistant/route` and
`assistant/shutdown` / `assistant/cancel_jobs` are bundled-worker compatibility extensions, both version 1.

Delivery uses durable acknowledgements. A crash in the small interval between
external notification delivery and SQLite acknowledgement can cause a repeated
notification; external desktop notifications and a local database cannot share
an atomic transaction. Acknowledged deliveries are deduplicated across restarts.

## Reminders and Notes

Say "set a timer for ten minutes" or "remind me tomorrow at nine AM to call".
Without AM/PM the plugin asks for clarification; reply AM or PM to resolve the pending request. Use timezone-aware RFC3339
`reminders.create` for specific dates. DST transitions require an explicit offset
when ambiguous. `reminders.list`, `cancel` and `snooze` use stable IDs. Recurrence
is deferred. Disable suspends delivery while retaining SQLite reminders.

Say "take a note buy batteries", "find my notes about batteries", "list my notes",
"read note ID", "replace note ID with TEXT", or "delete note ID". Read and create
are allowed for explicit requests; replacement and deletion require confirmation.
`notes.export` writes Markdown inside the plugin data directory. Search uses FTS5
with user words quoted as literal terms.

## Migration and troubleshooting

`nixe-assistant plugins migrate /path/to/checkout` creates descriptors and
separate state additively. Existing TOML files are not rewritten. Configured MCP
servers keep server IDs and `SERVER.TOOL` host names, startup environment,
permissions, dependencies and installation paths. DuckDuckGo's installed server
is adopted; its packages are not reinstalled. Browser profile directories and
search-provider settings remain unchanged. Migrated abilities retain host grants.

Use `doctor --json`, `plugins inspect ID`, and the Plugins tab to inspect failures.
Missing dependencies prevent activation. A process failure removes its tools from
catalogs and fails only that plugin. Fix the executable/schema and explicitly
reload. Notification failures keep reminders pending and emit an error. Plugin
changes do not reload the models. Updating ordinary config reloads compatibility
workers so their settings reflect the saved configuration.

Suggested future work: opt-in file indexing with private-directory exclusions,
clipboard/OCR tools, named routines, calendar/email connectors with review before
sending, media controls, and project-aware browser/coding workflows.

Protocol references: [MCP tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools) and [MCP lifecycle](https://modelcontextprotocol.io/specification/2025-11-25/basic/lifecycle).

## Desktop abilities and host coordination

See [Desktop plugins](DESKTOP-PLUGINS.md) for routines, project context, explicit
clipboard/screen capture, local calendar, opt-in file indexing, MPRIS media,
power controls, and conflict-safe undo. These packages use MCP tools; the host
owns coordinated dispatch and model lifetime. Stopping the daemon stops its HUD,
STT service, and plugin workers together.
