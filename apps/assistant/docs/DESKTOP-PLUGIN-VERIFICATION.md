# Desktop plugin verification — 2026-10-07

Implemented/deployed eight bundled packages: routines, projects, capture,
calendar, file-search, media, power, and history. Notes now provides conflict-safe
undo. The daemon owns coordinated dispatch, durable action history, and language
model lifetime.

- Workspace Rust tests passed (62 tests); affected tests were repeated after
  follow-up fixes. Formatting and Clippy with `-D warnings` passed.
- 14 Python tests passed, including real helper cancellation/reaping, SQLite
  persistence/conflicting undo, routine digest validation, exact project argument
  arrays, timezone-aware calendar/ICS export beyond 100 events, indexing
  exclusions/symlinks/stale results, temporary capture cleanup, and MPRIS calls.
- Actual MCP discovery passed for every bundled package. Input schemas, manifest
  tool declarations, permissions, and voice routes were checked together.
- Live deployment started 23 enabled plugins successfully; existing example.echo
  remained disabled. Configuration, apps, and commands stayed byte-identical.
- A disposable two-step routine displayed its exact steps for approval, invoked
  a note creation and clock lookup, and recorded the note's inverse operation.
  Shared undo restored the original state through the owner's confirmation.
- Disabling a routine while its note-edit child awaited approval cancelled that
  approval and preserved the note. The plugin was re-enabled afterward.
- Live project metadata, timezone-aware local calendar events, empty opt-in file
  roots, and MPRIS discovery were checked. Disposable active records were deleted,
  and their entries removed from the stopped host's action journal.
- Quiet idle unloading was exercised with a temporary two-second timeout.
  Battery settings persisted, normal settings were restored, and explicit unload
  reported the model absent.
- The `power-probe` example generated a fixed grammar response without microphone
  capture or playback, validating initial GPU/configured-device load, same-profile
  retention, unload, CPU lazy reload, CPU unload, configured-device lazy reload,
  and final unload.
- The Plugins TUI tab was inspected at 80×24. New packages appeared, and stopped
  services correctly showed saved enablement separately from running enablement.
- Resource reporting was checked live against service-owned process groups,
  including Python plugin processes and assistant-owned CUDA VRAM allocations.
- Final daemon/CLI/package contents match the final build and sources. All three
  assistant service units are inactive with PID zero; no daemon, STT, plugin
  workers, or assistant HUD layer remains.

The deployment preserved uncommitted work and used backups under
`~/.local/state/omarchy-assistant/backups/desktop-plugins-*`; the final deployment
backup is `desktop-plugins-20261007-160437`.

## Limits of verification and current scope

Clipboard/screen tests used fixtures and mocked capture commands, so no real
clipboard contents or screenshots were collected. Media discovery was live;
playback/volume changes were tested with command fixtures to avoid changing the
user's playback. No fresh physical microphone conversation was performed.

Calendar is local with RFC 5545 export; account synchronization, invitations,
and recurring events are not implemented. File search is intentionally
unconfigured until folders are selected. Undo covers notes, local calendar
changes, and routine/project definitions; terminal/coding-agent work, media,
browser actions, and resource controls do not provide inverse operations.
Power profiles control the language model, not unrelated GPU users or system-wide
performance policy. Stopping the daemon remains the complete resource shutdown.

## Reproduce

Run Python tests and the focused MCP integration test:

```bash
python3 -m unittest discover -s tests -v
cargo test -p assistant-tools --test plugins
```

Exercise the actual installed assistant with disposable records (temporarily
starts it, uses real approval IPC, and restores its initial service state):

```bash
scripts/verify-desktop-plugins.py
```

Probe model lifetime without starting the voice services:

```bash
CMAKE_CUDA_ARCHITECTURES=120a-real cargo run --release -p assistant-inference --example power-probe --features cuda
```
