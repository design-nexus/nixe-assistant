#!/usr/bin/env bash
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
bin_dir="${HOME}/.local/bin"
unit_dir="${HOME}/.config/systemd/user"
cfg_dir="${HOME}/.config/omarchy-assistant"
bind_file="${HOME}/.config/hypr/bindings.lua"

cd "$root"
echo "building omarchy-assistantd (release, CUDA)…"
cargo build --release -p omarchy-assistantd

mkdir -p "$bin_dir" "$unit_dir" "$cfg_dir"
install -m 0755 "$root/target/release/omarchy-assistantd" "$bin_dir/omarchy-assistantd"
install -m 0755 "$root/target/release/omarchy-assistant" "$bin_dir/omarchy-assistant"
# loqui/ort CUDA EP is loaded from the directory that contains the daemon binary
for so in libonnxruntime_providers_cuda.so libonnxruntime_providers_shared.so; do
  if [[ -e "$root/target/release/$so" ]]; then
    cp -a "$root/target/release/$so" "$bin_dir/$so"
  fi
done

if [[ ! -f "$cfg_dir/config.toml" ]]; then
  install -m 0644 "$root/config/config.toml" "$cfg_dir/config.toml"
fi
if [[ ! -f "$cfg_dir/voxtype-parakeet.toml" ]]; then
  install -m 0644 "$root/config/voxtype-parakeet.toml" "$cfg_dir/voxtype-parakeet.toml"
fi

install -m 0644 "$root/systemd/omarchy-assistantd.service" "$unit_dir/omarchy-assistantd.service"
install -m 0644 "$root/systemd/omarchy-assistant-hud.service" "$unit_dir/omarchy-assistant-hud.service"
install -m 0644 "$root/systemd/nyx-parakeet.service" "$unit_dir/nyx-parakeet.service"
systemctl --user daemon-reload
systemctl --user enable --now nyx-parakeet.service
systemctl --user enable --now omarchy-assistantd.service
systemctl --user enable --now omarchy-assistant-hud.service

if [[ -f "$bind_file" ]] && ! grep -q 'Nyx PTT' "$bind_file"; then
  {
    echo ""
    echo "-- Nyx push-to-talk (added by omarchy-assistant install.sh)"
    cat "$root/scripts/hypr-bind.lua"
  } >> "$bind_file"
  echo "added Alt+Space PTT bind to $bind_file"
  hyprctl reload >/dev/null 2>&1 || true
fi

echo "Nyx is installed. Hold Alt+Space to talk."
echo "logs: journalctl --user -u omarchy-assistantd -f"
echo "status: omarchy-assistant status"
echo "HUD: $root/scripts/hud.sh"
