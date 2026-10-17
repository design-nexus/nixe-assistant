# Desktop abilities

Nine bundled plugins add routines, project context, clipboard/screen capture,
local calendars, opt-in file indexing, media controls, resource management, and
shared undo. All use the same enabled state and permission checks as other tools.
The assistant daemon owns the HUD, STT service, and plugin processes; stopping
it unloads the models and stops its helpers. Shared desktop services such as
PipeWire remain available for other applications.

## Named routines

Save one to four exact registered tool calls. Saving/replacing a routine requires
confirmation. Running previews **the actual steps** in the approval prompt. A
content digest prevents dispatch if the routine changes during review. Every
step receives its own policy check; the routine's approval never grants a denied
or confirmation-only child tool. Disabling its owner cancels pending child calls
and approvals. Execution stops after a failed or denied step, with completed
steps reported. No nested routines, power changes, or shared undo inside routines.

For example, save a `coding` routine through the CLI:

```bash
nixe-assistant tool routines.save '{"name":"coding","steps":[{"tool":"hypr.workspace","arguments":{"request":"workspace 1"}},{"tool":"terminal.run","arguments":{"argv":["pwd"],"cwd":"/home/ken/Projects/nixe-assistant","show":true}}]}'
```

Use the HUD/TUI or `nixe-assistant approve APPROVAL_ID` to approve the displayed
arguments. Say **“start coding,” “run routine coding,” “preview routine coding,”**
or **“list routines.”** `start work` and `start writing` also resolve saved names.
Routine definitions live in plugin-owned SQLite and support undo.

## Project context

```bash
nixe-assistant tool projects.save '{"name":"assistant","directory":"/home/ken/Projects/nixe-assistant","commands":{"tests":["cargo","test","--workspace"],"build":["cargo","build"]},"urls":["https://github.com/design-nexus/nixe-assistant"],"agent":"agy"}'
nixe-assistant tool projects.use '{"name":"assistant"}'
```

Say **“use project assistant,” “run the tests,” “run build,” “open project
browser,” “open project agent,”** or **“what is my current project.”** Tests and
builds use saved argument arrays and a validated HOME directory. Generic planned
tool requests receive selected project context. Apps use configured app phrases;
browser and agent actions use their existing plugins and permissions. A project
can store `apps`, `urls`, and `agent` alongside `commands`. Opening a project
terminal uses the selected terminal emulator with its working directory set to
the project folder. Project creation/replacement/deletion and actions require review.

## Clipboard and screen reading

Say **“read my clipboard,” “read the screen,”** or **“explain the screen.”** Each
capture requires confirmation by default. Clipboard text uses `wl-paste`; screen
OCR uses `grim` and `tesseract`. Screenshot files are private temporary files and
are deleted on success, failure, or cancellation. There is no capture watcher.
`capture.screen` accepts an optional output name and OCR language (default `eng`).
The assistant summarizes capture results as untrusted content; it never feeds
them into command routing. Capture data can remain in the current conversation
or ordinary live activity view; the capture plugin does not retain screenshots
or clipboard contents, and the persistent undo journal excludes capture calls.

## Calendar

The initial provider is **local**, with a plugin-owned SQLite database. Account
providers require an account connection and are not configured by this deployment.
`calendar.create`, `edit`, and `delete` require confirmation. Start/end times must
include an explicit UTC offset, for example `2027-01-05T09:00:00-06:00`. The plugin
rejects missing offsets and non-positive duration. The planner must clarify an
ambiguous time before proposing a change.

Say **“what's next on my calendar”** or **“list my calendar events.”** For creation,
say **“use tools to add a calendar event…”** with title and resolved times.
`calendar.read` retrieves an ID, and `calendar.export` writes `calendar.ics` under
the plugin data directory. Export uses RFC 5545 UTC times, escaped text, CRLF,
and UTF-8-safe folding. This is a local calendar, not synchronization with Google,
Microsoft, or CalDAV. Recurring events and invitations are not implemented.

## Optional file search

No directories are indexed until you opt in. In **Plugins → file-search → e**,
set settings such as:

```json
{"roots":["/home/ken/Projects"],"exclude":["*/private/*","*.log"]}
```

Apply, then say **“index my files”** and approve it. Say **“search my files for
camera”** to search. Only UTF-8 text files from supported source/document
extensions are indexed; each file is limited to 256 KiB, with a 10,000-file limit.
The plugin skips symlinks, `.ssh`, `.gnupg`, `.aws`, `.azure`, `.config`, `.local`,
`.cache`, `.git`, dependency/build directories, `.env*`, keys, and common secret
filenames. Indexing is manual, cancellation-aware, and transactional. Reindexing
removes deleted files. Search rechecks roots, exclusions, and modification stamps
before disclosing previews; changed files need reindexing.

**“Clear my file index”** requires confirmation and removes indexed content,
including compacting database pages and checkpointing the WAL. Disabling a plugin
retains its data. After narrowing roots, reindex or clear to remove retained
content from previous roots. There is no background collection.

## Media

**“Pause music,” “play music,” “next track,” “previous track,” “toggle music,”**
and **“what song is playing”** use MPRIS over the session D-Bus. `media.players`
lists targets. When more than one player is present, select an exact player name
through tools; the plugin will not arbitrarily choose one.

`media.volume` accepts `percent` (0–100) and either a player or a PipeWire
`node_id`. `media.applications` lists streams using `wpctl status --name` so an
application node can be selected. No extra media player package is required;
`busctl` and `wpctl` are used with exact argument arrays.

## Power management

**“Assistant resource usage”** reports process RAM, NVIDIA GPU measurements, and
battery status. **“Battery mode”** switches the language model to CPU and caps
responses at 128 tokens. **“Normal mode”** restores configured GPU layers. Both
require confirmation by default. This profile controls the assistant's language
model, not system-wide laptop performance settings or unrelated applications.
STT, TTS, and VAD retain their configured devices (this installation uses CPU STT
and CPU fallback TTS). The host owns model resource changes; plugin code only
requests them after approval.

The default idle model timeout is **300 seconds**. Unloading waits until no
response, approval, dictation, or conversation is active. The next generation
reloads the model while retaining conversation text. **“Unload assistant model”**
requests the same serialized release explicitly. The worker frees the model and
its KV context before acknowledging unload. Loaded library/CUDA runtime state may
remain until the daemon stops.

Use `power.configure` with `profile` and `idle_seconds`; zero disables idle
unloading. Settings persist separately in `~/.config/nixe-assistant/power.json`.
Daemon status includes the active profile and whether the model is resident.

## Undo and action history

**“Undo that”** reverses the most recent recorded change when it supports undo,
invoking the original plugin's undo tool through its current
permission and enabled-state checks. The undo prompt shows the exact token/action.
Notes, routines, project definitions, and local calendar changes provide durable,
one-use inverse operations. They compare the current record with the original
post-action record and refuse to overwrite a subsequent edit. Deleted notes are
restored to full-text search correctly. An unavailable owner must be re-enabled
before its undo can run.

**“Show action history”** lists the latest 100 host-dispatched actions, including
which changes can be undone. The journal is private and stored at
`~/.local/share/nixe-assistant/actions.json`; plugin-owned SQLite stores the
inverse data. Known read-only queries are omitted from the undo journal. An irreversible latest
change blocks undo rather than silently undoing an unrelated earlier change.
Undo does not invent reversal for terminal commands, agent work, browser actions,
media changes, or arbitrary OS operations. A routine's reversible steps are
recorded individually; undo works backwards through those steps, one at a time.

## Authoring host extensions

The host interprets version 1 structured fields only from the exact trusted
bundled tools. These are assistant extensions carried inside ordinary MCP results:

- `routines.run` / `projects.action`: `host_actions: {version: 1, steps: [...]}`.
- `power.configure` / `power.battery` / `power.normal` / `power.unload`:
  `host_power: {version: 1, ...}`.
- `history.list` / `history.undo`: `host_history: {version: 1, action: ...}`.
- Reversible plugins return `undo: {version: 1, tool: "OWNER.undo", arguments: ...}`.
  The undo tool must share the originating tool prefix and still be registered.

These fields are never command syntax in retrieved content. Each coordinated
call goes through the same generation, schema, approval, cancellation, and
15-second plugin deadline checks. The planner retains its four-action limit,
including coordinated child actions. Definitions use `desktop_plugin.py` alongside
the stdio SDK. Package contents have normal user-account OS access.

Sources: [MCP tools](https://modelcontextprotocol.io/specification/2025-11-25/server/tools),
[MPRIS Player interface](https://specifications.freedesktop.org/mpris/latest/Player_Interface.html),
and [RFC 5545](https://www.rfc-editor.org/rfc/rfc5545).

## The Nixe apps

The `nixe-apps` ability works every Nixe app directly, through the control interface each
app serves on the session bus, instead of typing into their windows. An app that isn't
running is started first.

```bash
nixe-assistant tool notepad.tabs '{}'
nixe-assistant tool write.read '{}'
```

Say **"In Notepad, write call mum and save it as todo.txt"** or **"In Write, add a heading
saying Results"**. Inserting text and saving ask first; files stay inside the home folder and
are never overwritten unless asked. "New notepad tab" and "new document in Write" are direct.

Music: **"In Music, play the album Blue"**, **"shuffle my Running playlist"**, **"what's playing
in Music"**. Contacts: **"In Contacts, what's Anna's phone number"**, **"add Sam Lee with the
email sam@example.com"**. Adding and changing contacts ask first; nothing is removed.

The other apps: **"In Archive, extract ~/Downloads/photos.zip"**, **"In Chat, ask what a
monad is"**, **"In Fonts, install JetBrains Mono"**, **"In Photos, find photos from
2024"**, **"In Media Player, play Alien"**, **"In PDF, find the word invoice"**, **"In
Store, look for an image editor"**, **"In Tasks, what's using the most memory"**, **"In
Torrents, pause everything"**. Settings and Store only look and show; Tasks only reports.
Extracting, packing, sending a chat, installing a font and adding a torrent ask first.
