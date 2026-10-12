-- timers: "поставь таймер на 5 минут", "отмени таймер", "сколько осталось"

local id = jarvis.context.command_id

if id == "assistant_timer_cancel" then
  local n = jarvis.timer.cancel_all()
  if n > 0 then
    say(T("Таймер", "Timer"), T("Отменено: " .. n, "Cancelled: " .. n))
  else
    say(T("Таймер", "Timer"), T("Активных таймеров нет", "No active timers"))
  end
  return
end

if id == "assistant_timer_status" then
  local list = jarvis.timer.list()
  if #list == 0 then
    say(T("Таймер", "Timer"), T("Активных таймеров нет", "No active timers"))
    return
  end
  for _, t in ipairs(list) do
    say(T("Осталось ", "Left ") .. fmt_duration(math.max(1, t.remaining)), t.message, T("Осталось " .. fmt_duration(math.max(1, t.remaining)), "Time left: " .. fmt_duration(math.max(1, t.remaining))))
  end
  return
end

local secs = jarvis.parse.duration(phrase)
if not secs then
  say(T("Таймер", "Timer"), T("Скажите время, например: таймер на 5 минут", "Say the time, e.g. timer for 5 minutes"))
  return
end

local text = fmt_duration(secs)
jarvis.timer.after(secs, T("Таймер", "Timer"), T("Время вышло: " .. text, "Time is up: " .. text))
-- the shown text says when it rings (the clock of this computer); the spoken one stays short
local ok_at, at = pcall(function() return os.date("%H:%M:%S", os.time() + secs) end)
local when = (ok_at and at) and T(" — сработает в " .. at, " — rings at " .. at) or ""
say(T("Таймер", "Timer"), T("Запущен на " .. text, "Started for " .. text) .. when, T("Таймер на " .. text, "Timer for " .. text), "timer")
