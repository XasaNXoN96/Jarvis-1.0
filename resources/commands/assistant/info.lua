-- time, date, battery: "сколько времени", "какое сегодня число", "сколько заряда"

local id = jarvis.context.command_id
local t = jarvis.context.time

local WEEKDAYS_RU = { Monday = "понедельник", Tuesday = "вторник", Wednesday = "среда", Thursday = "четверг",
                      Friday = "пятница", Saturday = "суббота", Sunday = "воскресенье" }
local MONTHS_RU = { "января", "февраля", "марта", "апреля", "мая", "июня", "июля", "августа", "сентября",
                    "октября", "ноября", "декабря" }
local MONTHS_EN = { "January", "February", "March", "April", "May", "June", "July", "August", "September",
                    "October", "November", "December" }

if id == "assistant_info_time" then
  local h, m = tonumber(t.hour), tonumber(t.minute)
  local spoken
  if RU then
    spoken = "Сейчас " .. h .. " " .. plural_ru(h, "час", "часа", "часов")
    if m > 0 then spoken = spoken .. " " .. m .. " " .. plural_ru(m, "минута", "минуты", "минут") end
  else
    spoken = string.format("It is %d:%02d", h, m)
  end
  say(T("Время", "Time"), string.format("%02d:%02d", h, m), spoken)

elseif id == "assistant_info_date" then
  local day, month, year = tonumber(t.day), tonumber(t.month), tonumber(t.year)
  local text
  if RU then
    text = "Сегодня " .. (WEEKDAYS_RU[t.weekday] or t.weekday) .. ", " .. day .. " " .. MONTHS_RU[month] .. " " .. year .. " года"
  else
    text = "Today is " .. t.weekday .. ", " .. MONTHS_EN[month] .. " " .. day .. ", " .. year
  end
  say(T("Дата", "Date"), text)

elseif id == "assistant_info_battery" then
  local r = powershell("$b = Get-CimInstance Win32_Battery | Select-Object -First 1; if ($b) { \"$($b.EstimatedChargeRemaining)|$($b.BatteryStatus)\" }")
  local pct, status = (r.stdout or ""):match("(%d+)|(%d+)")
  if not pct then
    say(T("Батарея", "Battery"), T("Батарея не найдена (это стационарный компьютер)", "No battery found (desktop computer)"))
  else
    local charging = status == "2" or status == "6" or status == "7" or status == "8" or status == "9"
    local msg = pct .. "%" .. (charging and T(", заряжается", ", charging") or "")
    say(T("Батарея", "Battery"), msg,
        T("Заряд " .. pct .. " процентов" .. (charging and ", заряжается" or ""),
          "Battery at " .. pct .. " percent" .. (charging and ", charging" or "")))
  end
end
