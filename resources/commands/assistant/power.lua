-- power: "выключи компьютер через 10 минут", "перезагрузи компьютер", "отмени выключение"

local id = jarvis.context.command_id

if id == "assistant_power_cancel" then
  local r = run("shutdown", { "/a" })
  if r.success then
    say(T("Компьютер", "Computer"), T("Выключение отменено", "Shutdown cancelled"))
  else
    say(T("Компьютер", "Computer"), T("Нет запланированного выключения", "No shutdown is scheduled"))
  end
  return
end

-- hibernate and log off are immediate: no delay. The question «да / нет» is asked by Jarvis itself before the command runs
-- (these are high-risk commands: confirm.rs), so here the command only does what was confirmed
if id == "assistant_power_hibernate" or id == "assistant_power_logoff" then
  local flag = "/h"
  if id == "assistant_power_logoff" then flag = "/l" end
  say(T("Компьютер", "Computer"), flag == "/h" and T("Ухожу в гибернацию", "Hibernating") or T("Выхожу из системы", "Signing out"))
  local r = run("shutdown", { flag })
  if not r.success then
    say(T("Компьютер", "Computer"), T("Не удалось выполнить: ", "Failed: ") .. (r.stderr or ""))
  end
  return
end

local secs = jarvis.parse.duration(phrase)
local delayed = secs ~= nil
if not secs then
  local p = " " .. (phrase or ""):lower() .. " "
  if p:find("сейчас", 1, true) or p:find("немедленно", 1, true) or p:find("срочно", 1, true) or p:find(" now ", 1, true) or p:find("immediately", 1, true) then
    -- "сейчас": five seconds are left so that "отмени выключение" still works
    secs = 5
    delayed = true
  else
    -- no time given: leave a minute to change your mind (voice recognition can be wrong)
    secs = 60
  end
end

local flag, ask_ru, ask_en
if id == "assistant_power_restart" then
  flag, ask_ru, ask_en = "/r", "Перезагрузить компьютер", "Restart the computer"
else
  flag, ask_ru, ask_en = "/s", "Выключить компьютер", "Shut down the computer"
end

-- (the confirmation «да» was given before this point, see confirm.rs: a misheard phrase never gets here by itself)
-- clear a previous schedule first, otherwise Windows refuses with "already in progress"
run("shutdown", { "/a" })
local r = run("shutdown", { flag, "/t", tostring(secs), "/c", "Jarvis" })
if not r.success then
  say(T("Компьютер", "Computer"), T("Не удалось запланировать: ", "Could not schedule: ") .. (r.stderr or ""))
  return
end
local what_ru, what_en = "Выключение", "Shutdown"
if flag == "/r" then what_ru, what_en = "Перезагрузка", "Restart" end
local msg = T(what_ru .. " через " .. fmt_duration(secs), what_en .. " in " .. fmt_duration(secs))
if not delayed then
  msg = msg .. T(". Скажите «отмени выключение», чтобы отменить", ". Say \"cancel shutdown\" to cancel")
end
say(T("Компьютер", "Computer"), msg, nil, flag == "/s" and "power" or nil)
