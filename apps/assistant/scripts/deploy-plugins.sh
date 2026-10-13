#!/usr/bin/env bash
# Targeted initial plugin deployment; build and test before invoking.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
backup="$HOME/.local/state/omarchy-assistant/backups/plugins-$(date +%Y%m%d-%H%M%S)"
mkdir -p "$backup/files" "$HOME/.local/bin"
chmod 0700 "$backup"
assets=(.local/bin/omarchy-assistantd .local/bin/omarchy-assistant .local/bin/omarchy-assistant-tui .local/bin/omarchy-assistant-plugin-worker .config/omarchy-assistant .local/share/omarchy-assistant/abilities .local/share/omarchy-assistant/browser .local/share/omarchy-assistant/quickshell)
for asset in "${assets[@]}"; do
    if [[ -e "$HOME/$asset" ]]; then
        mkdir -p "$backup/files/$(dirname "$asset")"
        cp -a "$HOME/$asset" "$backup/files/$asset"
    fi
done
systemctl --user is-active omarchy-assistantd.service > "$backup/daemon-state" || true
rollback() {
    echo "Plugin deployment failed; restoring $backup" >&2
    systemctl --user stop omarchy-assistantd.service || true
    cp -a "$backup/files/." "$HOME/"
    if [[ $(cat "$backup/daemon-state") == active ]]; then
        systemctl --user restart omarchy-assistantd.service || true
    fi
}
trap rollback ERR
for binary in omarchy-assistantd omarchy-assistant omarchy-assistant-tui omarchy-assistant-plugin-worker; do
    install -m 0755 "$root/target/release/$binary" "$HOME/.local/bin/$binary.new"
    mv "$HOME/.local/bin/$binary.new" "$HOME/.local/bin/$binary"
done
# Existing config, models, browser profiles and search dependencies are retained.
"$HOME/.local/bin/omarchy-assistant" plugins migrate "$root"
systemctl --user restart omarchy-assistantd.service
healthy=0
for _ in {1..45}; do
    if "$HOME/.local/bin/omarchy-assistant" status --json > "$backup/health.json" 2>/dev/null && jq -e '.ok and .model_loaded and (.plugins|length>0) and ([.plugins[]|select(.saved.enabled and .health!="running")]|length==0)' "$backup/health.json" >/dev/null; then healthy=1; break; fi
    sleep 1
done
[[ $healthy == 1 ]]
trap - ERR
printf 'Plugin deployment healthy. Backup: %s\n' "$backup"
