-- small talk: "привет", "как дела", "кто ты", "что ты умеешь"

local id = jarvis.context.command_id
math.randomseed(tonumber(jarvis.context.time.timestamp) or 0)
math.random(); math.random()
local function pick(list) return list[math.random(#list)] end

if id == "assistant_greet" then
  local h = tonumber(jarvis.context.time.hour)
  local part_ru = (h >= 5 and h < 12) and "Доброе утро" or ((h >= 12 and h < 18) and "Добрый день" or ((h >= 18 and h < 23) and "Добрый вечер" or "Доброй ночи"))
  local part_en = (h >= 5 and h < 12) and "Good morning" or ((h >= 12 and h < 18) and "Good afternoon" or ((h >= 18 and h < 23) and "Good evening" or "Good night"))
  local tail = pick(RU and { "Чем могу помочь?", "К вашим услугам.", "Рад вас слышать." }
                       or { "How can I help?", "At your service.", "Good to hear you." })
  local text = T(part_ru, part_en) .. ", " .. T("сэр", "sir") .. ". " .. tail
  say(T("Джарвис", "Jarvis"), text)

elseif id == "assistant_howareyou" then
  say(T("Джарвис", "Jarvis"), pick(RU and {
    "Все системы в норме, сэр. А как вы?",
    "Лучше, чем вчера. Спасибо обновлениям.",
    "Отлично, сэр. Готов к работе.",
  } or {
    "All systems nominal, sir. And you?",
    "Better than yesterday, thanks to the updates.",
    "Excellent, sir. Ready to work.",
  }))

elseif id == "assistant_who" then
  say(T("Джарвис", "Jarvis"), T(
    "Я Джарвис, ваш голосовой помощник. Работаю без интернета и стараюсь не обижаться.",
    "I am Jarvis, your voice assistant. I work offline and try not to take offence."))

else -- help
  local lines = RU and {
    "таймер на 5 минут / напомни через 10 минут / разбуди завтра в 7:30",
    "громкость 50 процентов, громче, тише, выключи звук",
    "открой сайт ютуб, найди в гугле погоду, открой блокнот",
    "запиши идею ..., что я записывал",
    "пауза, следующий трек; сколько времени; сколько заряда",
    "озвучь этот текст (выделите текст мышкой)",
    "выключи компьютер через 10 минут, отмени выключение",
    "я играю, остановись через час; режим текста",
  } or {
    "timer for 5 minutes / remind me in 10 minutes / wake me tomorrow at 7:30",
    "volume 50 percent, louder, quieter, mute",
    "open website youtube, search for the weather, open notepad",
    "write down ..., read my notes",
    "pause, next track; what time is it; battery level",
    "read this (select some text with the mouse)",
    "shut down the computer in 10 minutes, cancel shutdown",
    "game mode for one hour; text mode",
  }
  jarvis.out(T("Что я умею:", "What I can do:"))
  for _, l in ipairs(lines) do jarvis.out("  - " .. l) end
  say(T("Помощь", "Help"), T("Список команд выведен в окне", "The list of commands is in the window"),
      T("Я умею ставить таймеры и напоминания, менять громкость, открывать сайты и программы, вести заметки и многое другое. Список в окне.",
        "I can set timers and reminders, change the volume, open sites and programs, keep notes and more. The list is in the window."))
end
