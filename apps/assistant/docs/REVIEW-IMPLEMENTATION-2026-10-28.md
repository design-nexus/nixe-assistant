# Review implementation — 2026-10-28

Addresses the 13 findings in REVIEW-2026-10-28.md. Gemma, Kokoro af_heart, local processing and notification preferences are preserved.

| Finding | Delivered change |
| --- | --- |
| Content logs | Removed normal transcript/reply/wake/prompt/command content logging. Explicit ten-minute diagnostic logging can be started and stopped from Settings. Existing journal entries remain. |
| Speech cleanup | Scope-owned cancellation and task abortion on success, errors and early returns. Failed playback is cleared. |
| False completion | Discovery is excluded from fulfillment. Empty tasks fail; visual and application requests require the corresponding output/action evidence. |
| Playback timeout | Wait for queue drain; detect five seconds without progress as an output failure. Cancellation clears the queue. |
| Sentence boundaries | Preserve common abbreviations and initials; include closing quotes in sentence boundaries. |
| Signed charts | Accept and draw signed bar values around a shared zero axis; labels preserve original values. |
| Unwanted Canvas | Only explicit validated Canvas output opens the window. Search tools no longer open it implicitly. |
| Reconnection | Visual client requests current settings/state/session/approvals. Timestamped states prevent older snapshots replacing newer state. |
| Discovery budget | Three bounded metadata discoveries separate from the configured action budget; schema selection stays bounded. |
| Aura overhead | Native compositor IPC and events, bounded requests, GTK socket wakeups, no 40 ms idle timer or repeated hyprctl subprocesses. Live display-mode/reduced-motion changes, explicit primary output, shared frame timing. |
| Media loading | At most two isolated image/PDF renderers, per-process resource limits and ten-second deadline; small PDF page cache; document reads off GTK. Image sizing updates when loaded. |
| Handoff provider | Unknown and uninstalled providers are rejected before dispatch; no silent Antigravity fallback. Existing launchers remain interactive. |
| Retention/continuity | Configurable 30-day retention and 256 MiB undo cap, review/search/clear controls, opt-in saved conversation history, explicit checkpoint continuation with file/config/capability checks and no replay of uncertain actions. |

Settings additions: General → Privacy and storage; Permissions → Capability health; Performance → Last failure; Activity → Resume from checkpoint; Aura → Primary output.

The compact failure record stores only stage/category, tool names, timing metadata and IDs. Tool schemas remain outside conversation history. Learned dictionary entries are preserved by history clearing.

CLI: `privacy status|prune|clear|failure|diagnostics on|off`, `history QUERY`, `tasks resume TASK_ID`. No model downloads or inference benchmarks are needed to use these controls.

## Validation

- 36 policy/retention tests passed, including preservation of dictionary entries and refusal to count discovery as completed work.
- 6 actual chunker tests passed, including character-streamed abbreviations and quotes.
- 3 planner tests passed, including paraphrased visual requests requiring actual Canvas output.
- 2 extracted actual speech-cleanup tests passed: error cancellation/abortion and successful conversation-token preservation.
- 2 extracted actual checkpoint tests passed: environment changes rejected, duplicate guard and restrictive trust preserved.
- Native media helper rendered a sample PNG and PDF with valid bounded frame output.
- Release builds for the Assistant workspace (CUDA enabled) and Settings passed. Both apps are installed; Assistant was restarted. All seven installed Assistant binaries and Settings match their release builds.
- Final live health: 268 tools, zero registry errors, Kokoro af_heart, desktop notifications off, saved conversation history off. Daemon and Aura startup journals have no warnings.
- Recovered a pre-existing corrupt history database: 438 audit records, 72 tasks and 131 task steps; integrity check passes. Original file retained at `~/.local/state/nyx-assistant/audit-recovery-y1vk6oyk/original.db`. No dictionary rows were recoverable; the known chalira → Calera correction was restored from prior session context.
- Restarted the stale accessibility bus; Aura registration now succeeds.

## Deliberate limits

- Physical microphone interruption, additional physical monitors, and fullscreen/scaling behavior still require hardware validation.
- One latest task checkpoint is retained, expires after 24 hours, and resumes only on explicit request. Unknown in-flight outcomes are refused rather than replayed; completed mutation digests remain protected. Changed settings, tools or relevant files require a fresh request. Window targeting continues to use normal live tool checks.
- Saved conversation history is opt-in and searchable; it is not silently injected into the model after restart.
- Managed noninteractive frontier delegation and new orchestration/model dependencies were not introduced. The audit called for keeping these separate from the terminal launcher.

## Files modified

Assistant repository:

- `apps/assistant/Cargo.lock`
- `apps/assistant/config/config.toml`
- `apps/assistant/crates/assistant-agent/src/chunker.rs`
- `apps/assistant/crates/assistant-audio/src/engine.rs`
- `apps/assistant/crates/assistant-audio/src/playq.rs`
- `apps/assistant/crates/assistant-core/src/config.rs`
- `apps/assistant/crates/assistant-core/src/lib.rs`
- `apps/assistant/crates/assistant-core/src/privacy.rs`
- `apps/assistant/crates/assistant-ipc/src/protocol.rs`
- `apps/assistant/crates/assistant-omarchy/src/lib.rs`
- `apps/assistant/crates/assistant-policy/src/store.rs`
- `apps/assistant/crates/assistant-policy/src/task.rs`
- `apps/assistant/crates/assistant-tools/src/agent.rs`
- `apps/assistant/crates/assistant-tools/src/fs.rs`
- `apps/assistant/crates/assistant-tools/src/registry.rs`
- `apps/assistant/crates/assistant-visual/Cargo.toml`
- `apps/assistant/crates/assistant-visual/src/main.rs`
- `apps/assistant/daemon/nyx-assistantd/src/app.rs`
- `apps/assistant/daemon/nyx-assistantd/src/checkpoint.rs`
- `apps/assistant/daemon/nyx-assistantd/src/cli.rs`
- `apps/assistant/daemon/nyx-assistantd/src/duplex.rs`
- `apps/assistant/daemon/nyx-assistantd/src/interaction.rs`
- `apps/assistant/daemon/nyx-assistantd/src/planner.rs`
- `apps/assistant/docs/REVIEW-2026-10-28.md`
- `apps/assistant/docs/REVIEW-IMPLEMENTATION-2026-10-28.md`

Settings repository:

- `apps/settings/src/sections/assistant/activity.rs`
- `apps/settings/src/sections/assistant/appearance.rs`
- `apps/settings/src/sections/assistant/general.rs`
- `apps/settings/src/sections/assistant/performance.rs`
- `apps/settings/src/sections/assistant/permissions.rs`
