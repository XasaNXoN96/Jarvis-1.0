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
