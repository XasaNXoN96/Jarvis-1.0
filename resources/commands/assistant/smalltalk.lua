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

else -- help: every command with example phrases
  jarvis.app.show_page("commands")           -- the window shows the full list with clickable phrases
  local list = jarvis.app.commands()
  jarvis.out(T("Все команды (" .. #list .. "):", "All commands (" .. #list .. "):"))
  local group
  for _, c in ipairs(list) do
    if c.group ~= group then
      group = c.group
      jarvis.out("")
      jarvis.out("[" .. group .. "]")
    end
    local examples = {}
    for i = 1, math.min(2, #c.phrases) do examples[#examples + 1] = "«" .. c.phrases[i] .. "»" end
    jarvis.out("  " .. table.concat(examples, ", "))
  end
  say(T("Помощь", "Help"), T("Список команд выведен, вкладка «Команды» открыта", "The command list is shown, the Commands tab is open"),
      T("Показываю все команды. Нажмите на фразу, чтобы вставить её.", "Showing all commands. Click a phrase to insert it."))
end
