-- game mode: "я играю, остановись через час", "игровой режим на 2 часа", "рабочий режим"

local id = jarvis.context.command_id

if id == "assistant_game_off" then
  jarvis.timer.cancel_all()
  run("taskkill", { "/f", "/im", "steam.exe" })
  say(T("Рабочий режим", "Work mode"), T("Таймеры отменены", "Timers cancelled"), T("Рабочий режим. Возвращаемся к делам.", "Work mode. Back to business."))
  return
end

local secs = jarvis.parse.duration(phrase)
local assumed = false
if not secs then
  secs = 3600
  assumed = true
end

local text = fmt_duration(secs)
jarvis.timer.after(secs, T("Игровой режим", "Game mode"), T("Время игры вышло. Пора заканчивать!", "Game time is up. Time to wrap up!"))
if secs > 900 then
  -- friendly heads-up before the end
  jarvis.timer.after(secs - 300, T("Игровой режим", "Game mode"), T("Осталось пять минут. Пора заканчивать.", "Five minutes left. Time to wrap up."))
end

local msg = T("Приятной игры. Напомню через " .. text, "Have fun. I'll remind you in " .. text)
if assumed then
  msg = msg .. T(" (время не названо, поставил час)", " (no time given, using one hour)")
end
say(T("Игровой режим", "Game mode"), msg, nil, "game")
