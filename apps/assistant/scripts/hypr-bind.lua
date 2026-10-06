-- Nyx push-to-talk. Paste into ~/.config/hypr/bindings.lua
-- Hold Alt+Space to talk; release to stop.
-- Super+Alt+Space stays the Omarchy Apps menu.
-- Repeating bind is the hold signal. A release bind on Alt+Space fires while
-- the chord is still down, so we do not use one.

local nyx = (os.getenv("HOME") or "") .. "/.local/bin/omarchy-assistant"
o.bind("ALT + SPACE", "Nyx PTT", nyx .. " ptt-start", { repeating = true })
