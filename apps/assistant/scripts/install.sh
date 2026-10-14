#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
bin_dir="$HOME/.local/bin"
unit_dir="$HOME/.config/systemd/user"
cfg_dir="$HOME/.config/nixe-assistant"
data_dir="$HOME/.local/share/nixe-assistant"
bind_file="$HOME/.config/hypr/bindings.lua"
cd "$root"
# Start it at the end only if it was running, or this is a first install.
start_after=0
if systemctl --user is-active --quiet nixe-assistantd.service omarchy-assistantd.service 2>/dev/null; then start_after=1; fi
if [[ ! -e $HOME/.config/nixe-assistant && ! -e $HOME/.config/omarchy-assistant ]]; then start_after=1; fi
# An install from before the rename moves over first.
"$root/scripts/migrate-from-omarchy.sh"
# Pin the browser runtime and retain its assets outside the checkout.
node_bin=$(readlink -f "$(command -v node)")
npm ci --prefix "$root/browser" --ignore-scripts
PLAYWRIGHT_BROWSERS_PATH="$HOME/.cache/nixe-assistant/chromium" "$node_bin" "$root/browser/node_modules/playwright/cli.js" install chromium
if [[ -f "$cfg_dir/build.env" ]]; then
  # Read only the supported override; never source executable config.
  arch=$(sed -n 's/^CMAKE_CUDA_ARCHITECTURES=//p' "$cfg_dir/build.env")
  if [[ -n $arch ]]; then export CMAKE_CUDA_ARCHITECTURES="$arch"; fi
fi
if [[ ${SKIP_BUILD:-0} != 1 ]]; then
  feature_args=()
  if command -v nvcc >/dev/null; then feature_args=(--features cuda); fi
  cargo build --release -p nixe-assistantd -p assistant-tui -p assistant-plugin-worker -p assistant-privileged "${feature_args[@]}"
fi
backup="$HOME/.local/state/nixe-assistant/backups/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$backup/files" "$cfg_dir" "$unit_dir" "$data_dir" "$bin_dir"
chmod 0700 "$backup"
assets=(.local/bin/nixe-assistant-plugin-worker .local/bin/nixe-assistant-helper .local/share/nixe-assistant/abilities .local/share/nixe-assistant/plugin-data .local/bin/nixe-assistant-node .local/share/nixe-assistant/browser .local/bin/nixe-assistantd .local/bin/nixe-assistant .local/bin/nixe-assistant-tui .local/bin/libonnxruntime_providers_cuda.so .local/bin/libonnxruntime_providers_shared.so .config/nixe-assistant .config/systemd/user/nixe-assistantd.service .config/systemd/user/nixe-assistant-hud.service .config/systemd/user/nixe-assistant-stt.service .local/share/nixe-assistant/quickshell .config/hypr/bindings.lua .config/omarchy/plugins .config/omarchy/shell.json)
for asset in "${assets[@]}"; do
  if [[ -e "$HOME/$asset" ]]; then mkdir -p "$backup/files/$(dirname "$asset")"; cp -a "$HOME/$asset" "$backup/files/$asset"; fi
done
systemctl --user is-active nixe-assistantd.service > "$backup/daemon-state" || true
rollback() {
  echo "Installation failed; restoring $backup" >&2
  systemctl --user stop nixe-assistantd.service nixe-assistant-hud.service || true
  cp -a "$backup/files/." "$HOME/"
  systemctl --user daemon-reload
  if [[ $(cat "$backup/daemon-state") == active ]]; then systemctl --user restart nixe-assistantd.service nixe-assistant-hud.service || true; fi
}
trap rollback ERR
for binary in nixe-assistantd nixe-assistant nixe-assistant-tui nixe-assistant-plugin-worker nixe-assistant-helper; do
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
printf '#!/usr/bin/env bash\nexec %q "$@"\n' "$node_bin" > "$bin_dir/nixe-assistant-node.new"
chmod 0755 "$bin_dir/nixe-assistant-node.new"
mv "$bin_dir/nixe-assistant-node.new" "$bin_dir/nixe-assistant-node"
"$bin_dir/nixe-assistant" migrate
"$bin_dir/nixe-assistant" plugins migrate "$root"
mkdir -p "$data_dir/quickshell"
cp -a "$root/quickshell/." "$data_dir/quickshell/"
for unit in nixe-assistantd nixe-assistant-hud nixe-assistant-stt; do install -m 0644 "$root/systemd/$unit.service" "$unit_dir/$unit.service"; done
"$root/scripts/install-plugin.sh"
# The admin tools' root helper and polkit policy live outside HOME, so they need sudo once,
# and again only when this build carries a newer one. Never fails the install.
helper=/usr/local/lib/nixe-assistant/nixe-assistant-helper
policy=/usr/share/polkit-1/actions/io.github.design_nexus.nixe.assistant.policy
want=$("$bin_dir/nixe-assistant-helper" --version)
if [[ $("$helper" --version 2>/dev/null || echo 0) -lt $want ]] || ! cmp -s "$policy" "$root/crates/assistant-privileged/data/io.github.design_nexus.nixe.assistant.policy"; then
  if [[ -t 0 ]] && sudo "$bin_dir/nixe-assistant" install-helper; then
    :
  else
    printf 'Admin tools (packages, system services) need their helper: sudo %s install-helper\n' "$bin_dir/nixe-assistant"
  fi
fi
systemctl --user daemon-reload
systemctl --user disable nixe-assistant-stt.service nixe-assistant-hud.service
systemctl --user enable nixe-assistantd.service
if [[ $start_after == 1 ]]; then systemctl --user restart nixe-assistantd.service; fi
if [[ -f "$bind_file" ]] && ! rg -q 'Assistant PTT' "$bind_file"; then
  cat "$root/scripts/hypr-bind.lua" >> "$bind_file"
  hyprctl reload
  [[ -z $(hyprctl configerrors) ]]
fi
if [[ $start_after != 1 ]]; then
  trap - ERR
  printf 'Installed (not started: it was stopped). Start it with: nixe-assistant start. Backup: %s\n' "$backup"
  exit 0
fi
healthy=0
for _ in {1..45}; do
  if "$bin_dir/nixe-assistant" status --json > "$backup/health.json" 2>/dev/null && jq -e '.ok and .model_loaded' "$backup/health.json" >/dev/null; then healthy=1; break; fi
  sleep 1
done
[[ $healthy == 1 ]]
trap - ERR
printf 'Installed and healthy. Backup: %s\n' "$backup"
