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

-- hibernate and log off are immediate: no delay, only the question
if id == "assistant_power_hibernate" or id == "assistant_power_logoff" then
  local flag, ask_ru, ask_en = "/h", "Отправить компьютер в гибернацию", "Hibernate the computer"
  if id == "assistant_power_logoff" then
    flag, ask_ru, ask_en = "/l", "Выйти из учётной записи Windows (несохранённые данные могут пропасть)", "Sign out of Windows (unsaved data may be lost)"
  end
  jarvis.session.start("confirm", 30)
  jarvis.session.set_data("power:" .. flag .. ":0:0")
  say(T("Компьютер", "Computer"), T(ask_ru .. "? Скажите «да» или «нет».", ask_en .. "? Say yes or no."),
    T(ask_ru .. "? Да или нет?", ask_en .. "? Yes or no?"))
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

-- nothing happens before the user says "да": the answer goes to the session (office.lua, confirm_session),
-- which schedules the shutdown. A misheard phrase can never turn the computer off by itself.
jarvis.session.start("confirm", 30)
jarvis.session.set_data("power:" .. flag .. ":" .. tostring(secs) .. ":" .. (delayed and "1" or "0"))
say(T("Компьютер", "Computer"),
  T(ask_ru .. " через " .. fmt_duration(secs) .. "? Скажите «да» или «нет».", ask_en .. " in " .. fmt_duration(secs) .. "? Say yes or no."),
  T(ask_ru .. " через " .. fmt_duration(secs) .. "? Да или нет?", ask_en .. " in " .. fmt_duration(secs) .. "? Yes or no?"))
