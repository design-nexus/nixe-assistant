#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
if systemctl --user is-active --quiet nixe-assistant-hud.service; then
  echo "HUD already running as nixe-assistant-hud.service" >&2
  exit 0
fi
exec qs --path "$root/quickshell" --daemonize --no-duplicate
