#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
plugin_id="${OMARCHY_ASSISTANT_PLUGIN_ID:-local.omarchy-assistant}"
# Keep the identifier already used by this user's bar.
if [[ -z ${OMARCHY_ASSISTANT_PLUGIN_ID:-} ]]; then
  for manifest in "$HOME"/.config/omarchy/plugins/*.omarchy-assistant/manifest.json; do
    if [[ -f $manifest ]]; then plugin_id=$(jq -r '.id' "$manifest"); break; fi
  done
fi
[[ "$plugin_id" =~ ^[a-zA-Z0-9_-]+\.omarchy-assistant$ ]] || { echo 'Invalid plugin ID' >&2; exit 1; }
target="$HOME/.config/omarchy/plugins/$plugin_id"
mkdir -p "$target"
cp -a "$root/plugins/local.omarchy-assistant/." "$target/"
python3 - "$target" "$plugin_id" <<'PY'
from pathlib import Path
import sys
for file in Path(sys.argv[1]).rglob('*'):
    if file.is_file():
        text=file.read_text()
        file.write_text(text.replace('local.omarchy-assistant', sys.argv[2]))
PY
omarchy-shell shell rescanPlugins
printf 'Installed plugin %s\n' "$plugin_id"
