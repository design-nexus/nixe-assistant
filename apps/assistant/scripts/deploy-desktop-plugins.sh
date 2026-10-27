#!/usr/bin/env bash
# Deploy built desktop abilities, retaining user data and initial daemon state.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
backup="$HOME/.local/state/nyx-assistant/backups/desktop-plugins-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$backup/files"
chmod 0700 "$backup"
systemctl --user is-active nyx-assistantd.service > "$backup/daemon-state" || true
systemctl --user stop nyx-assistantd.service
# The daemon owns all helpers. Wait for dependent units to finish stopping.
systemctl --user stop nyx-assistant-stt.service nyx-assistant-hud.service
assets=(.local/bin/nyx-assistantd .local/bin/nyx-assistant .local/bin/nyx-assistant-tui .local/bin/nyx-assistant-plugin-worker .config/nyx-assistant .local/share/nyx-assistant/abilities .local/share/nyx-assistant/plugin-data .local/share/nyx-assistant/actions.json)
for asset in "${assets[@]}"; do
    if [[ -e "$HOME/$asset" ]]; then
        mkdir -p "$backup/files/$(dirname "$asset")"
        cp -a "$HOME/$asset" "$backup/files/$asset"
    fi
done
rollback() {
    echo "Desktop plugin deployment failed; restoring $backup" >&2
    systemctl --user stop nyx-assistantd.service nyx-assistant-stt.service nyx-assistant-hud.service || true
    cp -a "$backup/files/." "$HOME/"
    if [[ $(cat "$backup/daemon-state") == active ]]; then systemctl --user start nyx-assistantd.service || true; fi
}
trap rollback ERR
for binary in nyx-assistantd nyx-assistant nyx-assistant-tui nyx-assistant-plugin-worker; do
    install -m 0755 "$root/target/release/$binary" "$HOME/.local/bin/$binary.new"
    mv "$HOME/.local/bin/$binary.new" "$HOME/.local/bin/$binary"
done
# Update bundled code while stopped; enabled state, grants, settings, and data remain separate.
for id in notes routines projects capture calendar file-search media power history; do
    mkdir -p "$HOME/.local/share/nyx-assistant/abilities/$id"
    cp -a "$root/abilities/$id/." "$HOME/.local/share/nyx-assistant/abilities/$id/"
done
"$HOME/.local/bin/nyx-assistant" plugins migrate "$root"
systemctl --user start nyx-assistantd.service
healthy=0
for _ in {1..45}; do
    if "$HOME/.local/bin/nyx-assistant" status --json > "$backup/health.json" 2>/dev/null && jq -e '.ok and .model_loaded and ([.plugins[]|select(.saved.enabled and .health!="running")]|length==0)' "$backup/health.json" >/dev/null; then healthy=1; break; fi
    sleep 1
done
[[ $healthy == 1 ]]
if [[ $(cat "$backup/daemon-state") != active ]]; then
    systemctl --user stop nyx-assistantd.service nyx-assistant-stt.service nyx-assistant-hud.service
fi
trap - ERR
printf 'Desktop plugins deployed. Initial daemon state restored. Backup: %s\n' "$backup"
