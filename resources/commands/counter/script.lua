-- simple counter demonstrating state persistence

local count = jarvis.state.get("count") or 0
count = count + 1
jarvis.state.set("count", count)

local lang = jarvis.context.language
local msg = lang == "ru"
    and "Счётчик: " .. count
    or "Counter: " .. count

jarvis.log("info", msg)
jarvis.system.notify("Counter", tostring(count))
jarvis.audio.play_ok()

return { chain = true }