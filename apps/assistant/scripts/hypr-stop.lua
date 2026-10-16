-- Nixe Assistant emergency stop. Paste into ~/.config/hypr/bindings.lua
-- Super+Shift+Escape stops speech, tasks, jobs and approvals and lets go of held keys,
-- whether or not the overlay is open. It never waits on the model.

local assistant_stop_path = (os.getenv("HOME") or "") .. "/.local/bin/nixe-assistant"
local assistant_stop = "'" .. assistant_stop_path:gsub("'", "'\\''") .. "'"
o.bind("SUPER + SHIFT + Escape", "Assistant stop", assistant_stop .. " cancel")
