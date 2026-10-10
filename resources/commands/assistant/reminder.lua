-- reminders and alarms:
--   "напомни через 5 минут позвонить маме", "поставь напоминание на 10 минут",
--   "скажи мне через 5 минут, чтобы я закончил игру", "остановись через 5 минут",
--   "напомни в 18:00 про тренировку", "разбуди меня завтра в 7:30"

local STOP = {
  "напомни", "напомнить", "напоминание", "напоминалку", "поставь", "поставить", "установи", "создай",
  "мне", "меня", "пожалуйста", "чтобы", "что", "бы", "я", "через", "на", "скажи", "сообщи", "предупреди",
  "разбуди", "разбудить", "будильник", "про", "о", "об",
  "remind", "me", "to", "that", "in", "set", "a", "reminder", "please", "tell", "warn", "wake", "up", "alarm", "about", "at",
}

local alarm = has_stem("разбуд") or has_stem("будильник") or has_stem("alarm") or has_stem("wake")

local secs, rest = jarvis.parse.duration(phrase)
local due, hhmm, days
if not secs then
  local unix, rest_clock
  unix, rest_clock, hhmm, days = jarvis.parse.clock(phrase)
  if unix then
    due, rest = unix, rest_clock
  end
end

if not secs and not due then
  say(T("Напоминание", "Reminder"),
      T("Скажите время, например: напомни через 5 минут или напомни в 18:00",
        "Say the time, e.g. remind me in 5 minutes or at 18:00"))
  return
end

local text = strip_words(rest or "", STOP)

local message
if text ~= "" then
  message = text
  if has_stem("чтобы") or has_stem("that") then
    -- "...чтобы я закончил игру": we cannot conjugate, so state it plainly
    message = T("Время вышло! Вы просили: ", "Time is up! You asked: ") .. text
  end
elseif alarm then
  message = T("Подъём, сэр! Время вставать.", "Wake up, sir! Time to get up.")
elseif has_stem("останов") or has_stem("заканч") or has_stem("прекрат") then
  message = T("Время вышло, пора заканчивать!", "Time is up, time to stop!")
else
  message = T("Напоминание!", "Reminder!")
end

local title = alarm and T("Будильник", "Alarm") or T("Напоминание", "Reminder")
local kind = alarm and "alarm" or "reminder"

if secs then
  jarvis.timer.after(secs, title, message)
  local when = fmt_duration(secs)
  say(title, T("Через " .. when .. ": " .. message, "In " .. when .. ": " .. message),
      T("Хорошо. Напомню через " .. when, "Alright. I'll remind you in " .. when), kind)
else
  jarvis.timer.at(due, title, message)
  local day_ru = days == 0 and "сегодня" or (days == 1 and "завтра" or (days == 2 and "послезавтра" or ""))
  local day_en = days == 0 and "today" or (days == 1 and "tomorrow" or (days == 2 and "the day after tomorrow" or ""))
  say(title, T(day_ru .. " в " .. hhmm .. ": " .. message, day_en .. " at " .. hhmm .. ": " .. message),
      T("Хорошо. Напомню " .. day_ru .. " в " .. hhmm, "Alright. I'll remind you " .. day_en .. " at " .. hhmm), kind)
end
