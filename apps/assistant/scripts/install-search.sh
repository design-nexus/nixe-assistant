#!/usr/bin/env bash
# Install the pinned AstroCorp MCP server with the reviewable local integration patch.
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
data="${XDG_DATA_HOME:-$HOME/.local/share}/omarchy-assistant/search"
revision=0cd5c5b9e8a4994736f18ef816c601dddf5a8324
node -e 'if(Number(process.versions.node.split(".")[0])<22)process.exit(1)'
mkdir -p "$data"
stage=$(mktemp -d "$data/.duckduckgo-XXXXXX")
git clone --quiet https://github.com/AstroCorp/MCP-DuckDuckGo.git "$stage"
git -C "$stage" checkout --quiet --detach "$revision"
git -C "$stage" apply "$root/search/astro-bounds.patch"
cp "$root/search/astro-package-lock.json" "$stage/package-lock.json"
npm ci --prefix "$stage" --ignore-scripts
npm run build --prefix "$stage"
node "$root/search/verify.mjs" "$stage"
if [[ -e "$data/duckduckgo" ]]; then
  backup="${XDG_STATE_HOME:-$HOME/.local/state}/omarchy-assistant/backups/$(date +%Y%m%d-%H%M%S)-search-assets"
  mkdir -m 0700 -p "$backup"
  mv "$data/duckduckgo" "$backup/duckduckgo"
  printf 'Previous search assets: %s\n' "$backup"
fi
mv "$stage" "$data/duckduckgo"
printf 'Installed DuckDuckGo MCP %s. Enable with: omarchy-assistant search-provider duckduckgo, then restart the daemon.\n' "$revision"
