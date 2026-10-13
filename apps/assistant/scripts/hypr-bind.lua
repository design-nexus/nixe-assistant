-- Omarchy Assistant push-to-talk. Paste into ~/.config/hypr/bindings.lua
-- Hold Alt+Space to talk; release to stop.
-- Super+Alt+Space stays the Omarchy Apps menu.
-- Repeating bind is the hold signal. A release bind on Alt+Space fires while
-- the chord is still down, so we do not use one.

local assistant_path = (os.getenv("HOME") or "") .. "/.local/bin/omarchy-assistant"
local assistant = "'" .. assistant_path:gsub("'", "'\\''") .. "'"
o.bind("ALT + SPACE", "Assistant PTT", assistant .. " ptt-start", { repeating = true })
