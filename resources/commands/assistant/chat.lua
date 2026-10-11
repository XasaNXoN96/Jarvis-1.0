-- "чат", "давай поговорим": a talk with the AI without the wake word until "закончи чат" (the session "chat";
-- the phrases themselves go to the AI in the application, the same way as typed questions)

local id = jarvis.context.command_id

if id == "assistant_chat_end" then
  if jarvis.session.kind() == "chat" then jarvis.session.finish() end
  say(T("Разговор", "Talk"), T("Режим разговора выключен", "Talk mode is off"))
  return
end

local ok, why = jarvis.ai.ready()
if not ok then
  say(T("Разговор", "Talk"), tostring(why), T("Режим разговора недоступен", "Talk mode is not available"))
  return
end
jarvis.session.start("chat", 180)
say(T("Разговор", "Talk"), T("Режим разговора включён: говорите без «Джарвис». Скажите «закончи чат», чтобы выйти.", "Talk mode is on: speak without the wake word. Say \"end chat\" to leave."),
    T("Слушаю. Говорите.", "I'm listening."))
