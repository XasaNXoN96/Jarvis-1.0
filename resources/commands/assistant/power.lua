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

local secs = jarvis.parse.duration(phrase)
local delayed = secs ~= nil
if not secs then
  -- no time given: leave a minute to change your mind (voice recognition can be wrong)
  secs = 60
end

local flag, what_ru, what_en
if id == "assistant_power_restart" then
  flag, what_ru, what_en = "/r", "Перезагрузка", "Restart"
else
  flag, what_ru, what_en = "/s", "Выключение", "Shutdown"
end

-- clear a previous schedule first, otherwise Windows refuses with "already in progress"
run("shutdown", { "/a" })
local r = run("shutdown", { flag, "/t", tostring(secs), "/c", "Jarvis" })
if not r.success then
  say(T("Компьютер", "Computer"), T("Не удалось запланировать: ", "Could not schedule: ") .. (r.stderr or ""))
  return
end

local msg = T(what_ru .. " через " .. fmt_duration(secs), what_en .. " in " .. fmt_duration(secs))
if not delayed then
  msg = msg .. T(". Скажите «отмени выключение», чтобы отменить", ". Say \"cancel shutdown\" to cancel")
end
say(T("Компьютер", "Computer"), msg, nil, id == "assistant_power_off" and "power" or nil)
