-- "что на экране", "опиши экран": a picture of the screen goes to the AI provider ONLY when "Анализ экрана" is allowed
-- in the settings (off by default) and the AI is switched on. The picture is not stored.

local question = phrase
if question == nil or question == "" or question:find("^%s*что на экран") or question:find("^%s*опиши экран") or question:find("^%s*посмотри на экран") then
  question = T("Что сейчас на экране? Опиши коротко главное.", "What is on the screen now? Describe the main things briefly.")
end

jarvis.out(T("Смотрю на экран…", "Looking at the screen…"))
local answer, why = jarvis.ai.screen(question)
if answer then
  say(T("Экран", "Screen"), answer)
else
  say(T("Экран", "Screen"), tostring(why), T("Не получилось посмотреть на экран", "Could not look at the screen"))
end
