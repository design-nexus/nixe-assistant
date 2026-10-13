#!/usr/bin/env bash
# Deploy built desktop abilities, retaining user data and initial daemon state.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
backup="$HOME/.local/state/nixe-assistant/backups/desktop-plugins-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$backup/files"
chmod 0700 "$backup"
systemctl --user is-active nixe-assistantd.service > "$backup/daemon-state" || true
systemctl --user stop nixe-assistantd.service
# The daemon owns all helpers. Wait for dependent units to finish stopping.
systemctl --user stop nixe-assistant-stt.service nixe-assistant-hud.service
assets=(.local/bin/nixe-assistantd .local/bin/nixe-assistant .local/bin/nixe-assistant-tui .local/bin/nixe-assistant-plugin-worker .config/nixe-assistant .local/share/nixe-assistant/abilities .local/share/nixe-assistant/plugin-data .local/share/nixe-assistant/actions.json)
for asset in "${assets[@]}"; do
    if [[ -e "$HOME/$asset" ]]; then
        mkdir -p "$backup/files/$(dirname "$asset")"
        cp -a "$HOME/$asset" "$backup/files/$asset"
    fi
done
rollback() {
    echo "Desktop plugin deployment failed; restoring $backup" >&2
    systemctl --user stop nixe-assistantd.service nixe-assistant-stt.service nixe-assistant-hud.service || true
    cp -a "$backup/files/." "$HOME/"
    if [[ $(cat "$backup/daemon-state") == active ]]; then systemctl --user start nixe-assistantd.service || true; fi
}
trap rollback ERR
for binary in nixe-assistantd nixe-assistant nixe-assistant-tui nixe-assistant-plugin-worker; do
    install -m 0755 "$root/target/release/$binary" "$HOME/.local/bin/$binary.new"
    mv "$HOME/.local/bin/$binary.new" "$HOME/.local/bin/$binary"
done
# Update bundled code while stopped; enabled state, grants, settings, and data remain separate.
for id in notes routines projects capture calendar file-search media power history; do
    mkdir -p "$HOME/.local/share/nixe-assistant/abilities/$id"
    cp -a "$root/abilities/$id/." "$HOME/.local/share/nixe-assistant/abilities/$id/"
done
"$HOME/.local/bin/nixe-assistant" plugins migrate "$root"
systemctl --user start nixe-assistantd.service
healthy=0
for _ in {1..45}; do
    if "$HOME/.local/bin/nixe-assistant" status --json > "$backup/health.json" 2>/dev/null && jq -e '.ok and .model_loaded and ([.plugins[]|select(.saved.enabled and .health!="running")]|length==0)' "$backup/health.json" >/dev/null; then healthy=1; break; fi
    sleep 1
done
[[ $healthy == 1 ]]
if [[ $(cat "$backup/daemon-state") != active ]]; then
    systemctl --user stop nixe-assistantd.service nixe-assistant-stt.service nixe-assistant-hud.service
fi
trap - ERR
printf 'Desktop plugins deployed. Initial daemon state restored. Backup: %s\n' "$backup"
