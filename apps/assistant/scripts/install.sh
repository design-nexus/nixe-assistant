#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
bin_dir="$HOME/.local/bin"
unit_dir="$HOME/.config/systemd/user"
cfg_dir="$HOME/.config/omarchy-assistant"
data_dir="$HOME/.local/share/omarchy-assistant"
bind_file="$HOME/.config/hypr/bindings.lua"
cd "$root"
# Pin the browser runtime and retain its assets outside the checkout.
node_bin=$(readlink -f "$(command -v node)")
npm ci --prefix "$root/browser" --ignore-scripts
PLAYWRIGHT_BROWSERS_PATH="$HOME/.cache/omarchy-assistant/chromium" "$node_bin" "$root/browser/node_modules/playwright/cli.js" install chromium
if [[ -f "$cfg_dir/build.env" ]]; then
  # Read only the supported override; never source executable config.
  arch=$(sed -n 's/^CMAKE_CUDA_ARCHITECTURES=//p' "$cfg_dir/build.env")
  if [[ -n $arch ]]; then export CMAKE_CUDA_ARCHITECTURES="$arch"; fi
fi
if [[ ${SKIP_BUILD:-0} != 1 ]]; then
  feature_args=()
  if command -v nvcc >/dev/null; then feature_args=(--features cuda); fi
  cargo build --release -p omarchy-assistantd -p assistant-tui -p assistant-plugin-worker "${feature_args[@]}"
fi
backup="$HOME/.local/state/omarchy-assistant/backups/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$backup/files" "$cfg_dir" "$unit_dir" "$data_dir" "$bin_dir"
chmod 0700 "$backup"
assets=(.local/bin/omarchy-assistant-plugin-worker .local/share/omarchy-assistant/abilities .local/share/omarchy-assistant/plugin-data .local/bin/omarchy-assistant-node .local/share/omarchy-assistant/browser .local/bin/omarchy-assistantd .local/bin/omarchy-assistant .local/bin/omarchy-assistant-tui .local/bin/libonnxruntime_providers_cuda.so .local/bin/libonnxruntime_providers_shared.so .config/omarchy-assistant .config/systemd/user/omarchy-assistantd.service .config/systemd/user/omarchy-assistant-hud.service .config/systemd/user/nyx-parakeet.service .local/share/omarchy-assistant/quickshell .config/hypr/bindings.lua .config/omarchy/plugins .config/omarchy/shell.json)
for asset in "${assets[@]}"; do
  if [[ -e "$HOME/$asset" ]]; then mkdir -p "$backup/files/$(dirname "$asset")"; cp -a "$HOME/$asset" "$backup/files/$asset"; fi
done
systemctl --user is-active omarchy-assistantd.service > "$backup/daemon-state" || true
rollback() {
  echo "Installation failed; restoring $backup" >&2
  systemctl --user stop omarchy-assistantd.service omarchy-assistant-hud.service || true
  cp -a "$backup/files/." "$HOME/"
  systemctl --user daemon-reload
  if [[ $(cat "$backup/daemon-state") == active ]]; then systemctl --user restart omarchy-assistantd.service omarchy-assistant-hud.service || true; fi
}
trap rollback ERR
for binary in omarchy-assistantd omarchy-assistant omarchy-assistant-tui omarchy-assistant-plugin-worker; do
  install -m 0755 "$root/target/release/$binary" "$bin_dir/$binary.new"
  mv "$bin_dir/$binary.new" "$bin_dir/$binary"
done
for so in libonnxruntime_providers_cuda.so libonnxruntime_providers_shared.so; do
  if [[ -e "$root/target/release/$so" ]]; then cp -a "$root/target/release/$so" "$bin_dir/$so"; fi
done
for name in config apps commands voxtype-parakeet; do
  if [[ ! -f "$cfg_dir/$name.toml" ]]; then install -m 0644 "$root/config/$name.toml" "$cfg_dir/$name.toml"; fi
done
mkdir -p "$data_dir/browser"
cp -a "$root/browser/." "$data_dir/browser/"
printf '#!/usr/bin/env bash\nexec %q "$@"\n' "$node_bin" > "$bin_dir/omarchy-assistant-node.new"
chmod 0755 "$bin_dir/omarchy-assistant-node.new"
mv "$bin_dir/omarchy-assistant-node.new" "$bin_dir/omarchy-assistant-node"
"$bin_dir/omarchy-assistant" migrate
"$bin_dir/omarchy-assistant" plugins migrate "$root"
mkdir -p "$data_dir/quickshell"
cp -a "$root/quickshell/." "$data_dir/quickshell/"
for unit in omarchy-assistantd omarchy-assistant-hud nyx-parakeet; do install -m 0644 "$root/systemd/$unit.service" "$unit_dir/$unit.service"; done
"$root/scripts/install-plugin.sh"
systemctl --user daemon-reload
systemctl --user disable nyx-parakeet.service omarchy-assistant-hud.service
systemctl --user enable omarchy-assistantd.service
systemctl --user restart omarchy-assistantd.service
if [[ -f "$bind_file" ]] && ! rg -q 'Assistant PTT' "$bind_file"; then
  cat "$root/scripts/hypr-bind.lua" >> "$bind_file"
  hyprctl reload
  [[ -z $(hyprctl configerrors) ]]
fi
healthy=0
for _ in {1..45}; do
  if "$bin_dir/omarchy-assistant" status --json > "$backup/health.json" 2>/dev/null && jq -e '.ok and .model_loaded' "$backup/health.json" >/dev/null; then healthy=1; break; fi
  sleep 1
done
[[ $healthy == 1 ]]
trap - ERR
printf 'Installed and healthy. Backup: %s\n' "$backup"
