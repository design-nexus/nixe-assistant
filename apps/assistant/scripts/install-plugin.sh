#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
plugin_id="${NYX_ASSISTANT_PLUGIN_ID:-local.nyx-assistant}"
# Keep the identifier already used by this user's bar.
if [[ -z ${NYX_ASSISTANT_PLUGIN_ID:-} ]]; then
  for manifest in "$HOME"/.config/omarchy/plugins/*.nyx-assistant/manifest.json; do
    if [[ -f $manifest ]]; then plugin_id=$(jq -r '.id' "$manifest"); break; fi
  done
fi
[[ "$plugin_id" =~ ^[a-zA-Z0-9_-]+\.nyx-assistant$ ]] || { echo 'Invalid plugin ID' >&2; exit 1; }
target="$HOME/.config/omarchy/plugins/$plugin_id"
mkdir -p "$target"
cp -a "$root/plugins/local.nyx-assistant/." "$target/"
python3 - "$target" "$plugin_id" <<'PY'
from pathlib import Path
import sys
for file in Path(sys.argv[1]).rglob('*'):
    if file.is_file():
        text=file.read_text()
        file.write_text(text.replace('local.nyx-assistant', sys.argv[2]))
PY
omarchy-shell shell rescanPlugins
printf 'Installed plugin %s\n' "$plugin_id"
