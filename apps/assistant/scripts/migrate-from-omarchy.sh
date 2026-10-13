#!/usr/bin/env bash
# Moves an install from its old name (omarchy-assistant, with the nyx-parakeet
# speech service) to Nixe Assistant: stops the old services, moves the folders
# (leaving links behind), renames units, binds and the bar plugin, and removes
# the old programs. Runs once; does nothing when there's nothing old.
set -euo pipefail
unit_dir="$HOME/.config/systemd/user"

[[ -d $HOME/.config/omarchy-assistant && ! -L $HOME/.config/omarchy-assistant && ! -e $HOME/.config/nixe-assistant ]] || exit 0
echo "Moving the assistant from omarchy-assistant to nixe-assistant"

systemctl --user disable --now omarchy-assistantd.service omarchy-assistant-hud.service nyx-parakeet.service 2>/dev/null || true

for base in .config .local/share .local/state .cache; do
  old="$HOME/$base/omarchy-assistant" new="$HOME/$base/nixe-assistant"
  if [[ -d $old && ! -L $old && ! -e $new ]]; then
    mv "$old" "$new"
    ln -s nixe-assistant "$old"
  fi
done

# Program names inside its own settings files.
for f in "$HOME"/.config/nixe-assistant/*.toml; do
  [[ -f $f ]] && sed -i 's/omarchy-assistant/nixe-assistant/g; s/nyx-parakeet/nixe-assistant-stt/g' "$f"
done

# Units: the CUDA drop-in comes along; the old units and programs go.
if [[ -d $unit_dir/omarchy-assistantd.service.d && ! -e $unit_dir/nixe-assistantd.service.d ]]; then
  mv "$unit_dir/omarchy-assistantd.service.d" "$unit_dir/nixe-assistantd.service.d"
fi
rm -f "$unit_dir"/omarchy-assistantd.service "$unit_dir"/omarchy-assistant-hud.service "$unit_dir"/nyx-parakeet.service
systemctl --user daemon-reload
for b in omarchy-assistant omarchy-assistantd omarchy-assistant-tui omarchy-assistant-plugin-worker omarchy-assistant-node omarchy-assistant-browser; do
  rm -f "$HOME/.local/bin/$b"
done

# Hyprland binds that run it.
bind_file="$HOME/.config/hypr/bindings.lua"
if [[ -f $bind_file ]] && grep -q 'omarchy-assistant' "$bind_file"; then
  cp -a "$bind_file" "$bind_file.nixe-bak"
  sed -i 's/omarchy-assistant/nixe-assistant/g' "$bind_file"
fi

# The bar plugin (<you>.omarchy-assistant) and its place in the bar.
shell_json="$HOME/.config/omarchy/shell.json"
for dir in "$HOME"/.config/omarchy/plugins/*.omarchy-assistant; do
  [[ -d $dir ]] || continue
  old_id=$(basename "$dir")
  new_id="${old_id%.omarchy-assistant}.nixe-assistant"
  [[ -e $HOME/.config/omarchy/plugins/$new_id ]] && continue
  mv "$dir" "$HOME/.config/omarchy/plugins/$new_id"
  rm -f "$HOME/.config/omarchy/plugins/$new_id/bin/nyx-bar-action"
  grep -rl "$old_id\|omarchy-assistant" "$HOME/.config/omarchy/plugins/$new_id" 2>/dev/null |
    xargs -r sed -i "s/$old_id/$new_id/g; s/omarchy-assistant/nixe-assistant/g"
  if [[ -f $shell_json ]] && grep -q "$old_id" "$shell_json"; then
    cp -a "$shell_json" "$shell_json.nixe-bak"
    sed -i "s/$old_id/$new_id/g" "$shell_json"
  fi
done
echo "Moved. Old settings are linked, so anything still using the old paths keeps working."
