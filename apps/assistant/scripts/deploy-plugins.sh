#!/usr/bin/env bash
# Targeted initial plugin deployment; build and test before invoking.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
backup="$HOME/.local/state/nixe-assistant/backups/plugins-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$backup/files" "$HOME/.local/bin"
chmod 0700 "$backup"
assets=(.local/bin/nixe-assistantd .local/bin/nixe-assistant .local/bin/nixe-assistant-tui .local/bin/nixe-assistant-plugin-worker .config/nixe-assistant .local/share/nixe-assistant/abilities .local/share/nixe-assistant/browser .local/share/nixe-assistant/quickshell)
for asset in "${assets[@]}"; do
    if [[ -e "$HOME/$asset" ]]; then
        mkdir -p "$backup/files/$(dirname "$asset")"
        cp -a "$HOME/$asset" "$backup/files/$asset"
    fi
done
systemctl --user is-active nixe-assistantd.service > "$backup/daemon-state" || true
rollback() {
    echo "Plugin deployment failed; restoring $backup" >&2
    systemctl --user stop nixe-assistantd.service || true
    cp -a "$backup/files/." "$HOME/"
    if [[ $(cat "$backup/daemon-state") == active ]]; then
        systemctl --user restart nixe-assistantd.service || true
    fi
}
trap rollback ERR
for binary in nixe-assistantd nixe-assistant nixe-assistant-tui nixe-assistant-plugin-worker; do
    install -m 0755 "$root/target/release/$binary" "$HOME/.local/bin/$binary.new"
    mv "$HOME/.local/bin/$binary.new" "$HOME/.local/bin/$binary"
done
# Existing config, models, browser profiles and search dependencies are retained.
"$HOME/.local/bin/nixe-assistant" plugins migrate "$root"
systemctl --user restart nixe-assistantd.service
healthy=0
for _ in {1..45}; do
    if "$HOME/.local/bin/nixe-assistant" status --json > "$backup/health.json" 2>/dev/null && jq -e '.ok and .model_loaded and (.plugins|length>0) and ([.plugins[]|select(.saved.enabled and .health!="running")]|length==0)' "$backup/health.json" >/dev/null; then healthy=1; break; fi
    sleep 1
done
[[ $healthy == 1 ]]
trap - ERR
printf 'Plugin deployment healthy. Backup: %s\n' "$backup"
