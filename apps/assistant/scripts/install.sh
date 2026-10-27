#!/usr/bin/env bash
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
bin_dir="$HOME/.local/bin"
unit_dir="$HOME/.config/systemd/user"
cfg_dir="$HOME/.config/nyx-assistant"
data_dir="$HOME/.local/share/nyx-assistant"
bind_file="$HOME/.config/hypr/bindings.lua"
cd "$root"
# Start it at the end only if it was running, or this is a first install.
start_after=0
# Migration installs preserve stopped/running state without restarting.
if systemctl --user is-active --quiet nyx-assistantd.service omarchy-assistantd.service 2>/dev/null; then :; fi
if [[ ! -e $HOME/.config/nyx-assistant && ! -e $HOME/.config/omarchy-assistant ]]; then :; fi
# An install from before the rename moves over first.
"$root/scripts/migrate-from-omarchy.sh"
# Pin the browser runtime and retain its assets outside the checkout.
node_bin=$(readlink -f "$(command -v node)")
[[ -d "$root/browser/node_modules/playwright" ]] || npm ci --prefix "$root/browser" --ignore-scripts
# Browser download stays an explicit later runtime setup operation.
if [[ -f "$cfg_dir/build.env" ]]; then
  # Read only the supported override; never source executable config.
  arch=$(sed -n 's/^CMAKE_CUDA_ARCHITECTURES=//p' "$cfg_dir/build.env")
  if [[ -n $arch ]]; then export CMAKE_CUDA_ARCHITECTURES="$arch"; fi
fi
if [[ ${SKIP_BUILD:-0} != 1 ]]; then
  feature_args=()
  if command -v nvcc >/dev/null; then feature_args=(--features cuda); fi
  cargo build --release --locked -p nyx-assistantd -p assistant-tui -p assistant-plugin-worker -p assistant-privileged -p assistant-asr -p assistant-visual "${feature_args[@]}"
fi
backup="$HOME/.local/state/nyx-assistant/backups/$(date +%Y%m%d-%H%M%S)"
mkdir -p "$backup/files" "$cfg_dir" "$unit_dir" "$data_dir" "$bin_dir"
chmod 0700 "$backup"
assets=(.local/bin/nyx-assistant-plugin-worker .local/bin/nyx-assistant-helper .local/bin/nyx-assistant-asr .local/bin/nyx-assistant-visual .local/share/nyx-assistant/abilities .local/share/nyx-assistant/plugin-data .local/bin/nyx-assistant-node .local/share/nyx-assistant/browser .local/bin/nyx-assistantd .local/bin/nyx-assistant .local/bin/nyx-assistant-tui .local/bin/libonnxruntime_providers_cuda.so .local/bin/libonnxruntime_providers_shared.so .config/nyx-assistant .config/systemd/user/nyx-assistantd.service .config/systemd/user/nyx-assistant-hud.service .config/systemd/user/nyx-assistant-visual.service .config/systemd/user/nyx-assistant-stt.service .config/systemd/user/nyx-assistant-failed.service .config/systemd/user/nyx-assistant-stt-failed.service .local/share/nyx-assistant/quickshell .config/hypr/bindings.lua .config/omarchy/plugins .config/omarchy/shell.json)
for asset in "${assets[@]}"; do
  if [[ -e "$HOME/$asset" ]]; then mkdir -p "$backup/files/$(dirname "$asset")"; cp -a "$HOME/$asset" "$backup/files/$asset"; fi
done
systemctl --user is-active nyx-assistantd.service > "$backup/daemon-state" || true
rollback() {
  echo "Installation failed; restoring $backup" >&2
  # Restore files without stopping the running Assistant.
  cp -a "$backup/files/." "$HOME/"
  systemctl --user daemon-reload
  # Keep the pre-install running process unchanged.
}
trap rollback ERR
for binary in nyx-assistantd nyx-assistant nyx-assistant-tui nyx-assistant-plugin-worker nyx-assistant-helper nyx-assistant-asr nyx-assistant-visual; do
  install -m 0755 "${CARGO_TARGET_DIR:-$root/target}/release/$binary" "$bin_dir/$binary.new"
  mv "$bin_dir/$binary.new" "$bin_dir/$binary"
done
for so in libonnxruntime_providers_cuda.so libonnxruntime_providers_shared.so; do
  if [[ -e "${CARGO_TARGET_DIR:-$root/target}/release/$so" ]]; then cp -a "${CARGO_TARGET_DIR:-$root/target}/release/$so" "$bin_dir/$so"; fi
done
for name in config apps commands voxtype-parakeet; do
  if [[ ! -f "$cfg_dir/$name.toml" ]]; then install -m 0644 "$root/config/$name.toml" "$cfg_dir/$name.toml"; fi
done
mkdir -p "$data_dir/browser"
cp -a "$root/browser/." "$data_dir/browser/"
mkdir -p "$data_dir/voice-engines/chatterbox"
install -m 0644 "$root/voice-workers/chatterbox.py" "$data_dir/voice-engines/chatterbox/worker.py"
printf '#!/usr/bin/env bash\nexec %q "$@"\n' "$node_bin" > "$bin_dir/nyx-assistant-node.new"
chmod 0755 "$bin_dir/nyx-assistant-node.new"
mv "$bin_dir/nyx-assistant-node.new" "$bin_dir/nyx-assistant-node"
"$bin_dir/nyx-assistant" migrate
"$bin_dir/nyx-assistant" persona >/dev/null
"$bin_dir/nyx-assistant" plugins migrate "$root"
mkdir -p "$data_dir/quickshell"
cp -a "$root/quickshell/." "$data_dir/quickshell/"
for unit in nyx-assistantd nyx-assistant-hud nyx-assistant-visual nyx-assistant-stt nyx-assistant-failed nyx-assistant-stt-failed; do install -m 0644 "$root/systemd/$unit.service" "$unit_dir/$unit.service"; done
"$root/scripts/install-plugin.sh"
# The streaming speech model is a separate, explicit download; without it, speech falls back to Voxtype.
if [[ ! -f "$data_dir/models/sherpa-onnx-nemotron-speech-streaming-en-0.6b-560ms-int8-2026-04-25/encoder.int8.onnx" ]]; then
  printf 'Streaming speech (listening while it speaks) needs its model: %s models fetch asr\n' "$bin_dir/nyx-assistant"
fi
# The admin tools' root helper and polkit policy live outside HOME, so they need sudo once,
# and again only when this build carries a newer one. Never fails the install.
helper=/usr/local/lib/nyx-assistant/nyx-assistant-helper
policy=/usr/share/polkit-1/actions/io.github.design_nexus.nyx.assistant.policy
want=$("$bin_dir/nyx-assistant-helper" --version)
if [[ $("$helper" --version 2>/dev/null || echo 0) -lt $want ]] || ! cmp -s "$policy" "$root/crates/assistant-privileged/data/io.github.design_nexus.nyx.assistant.policy"; then
  if [[ -t 0 ]] && sudo "$bin_dir/nyx-assistant" install-helper; then
    :
  else
    printf 'Admin tools (packages, system services) need their helper: sudo %s install-helper\n' "$bin_dir/nyx-assistant"
  fi
fi
systemctl --user daemon-reload
systemctl --user enable nyx-assistant-visual.service
# Preserve existing service enablement during migration.
# Existing daemon enablement is retained.
if [[ $start_after == 1 ]]; then systemctl --user restart nyx-assistantd.service; fi
if [[ ${NYX_SKIP_INTEGRATION:-0} != 1 && -f "$bind_file" ]]; then
  bound=0
  if ! rg -q 'Assistant PTT' "$bind_file"; then cat "$root/scripts/hypr-bind.lua" >> "$bind_file"; bound=1; fi
  if ! rg -q 'Assistant stop' "$bind_file"; then printf '\n' >> "$bind_file"; cat "$root/scripts/hypr-stop.lua" >> "$bind_file"; bound=1; fi
  if [[ $bound == 1 ]]; then
    hyprctl reload
    [[ -z $(hyprctl configerrors) ]]
  fi
fi
if [[ $start_after != 1 ]]; then
  trap - ERR
  printf 'Installed without starting or restarting Assistant. Backup: %s\n' "$backup"
  exit 0
fi
healthy=0
for _ in {1..45}; do
  if "$bin_dir/nyx-assistant" status --json > "$backup/health.json" 2>/dev/null && jq -e '.ok and .model_loaded' "$backup/health.json" >/dev/null; then healthy=1; break; fi
  sleep 1
done
[[ $healthy == 1 ]]
trap - ERR
printf 'Installed and healthy. Backup: %s\n' "$backup"
