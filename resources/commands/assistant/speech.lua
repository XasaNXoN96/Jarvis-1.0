-- speech: "озвучь этот текст" (reads what you have selected), "прочитай буфер",
--         "хватит читать", "выключи озвучку", "включи озвучку"

local id = jarvis.context.command_id

if id == "assistant_tts_test" then
  -- lists what can speak on this computer in the Jarvis window and says a test phrase
  jarvis.tts.set_enabled(true)
  jarvis.tts.diagnose()
  return
end

if id == "assistant_tts_stop" then
  jarvis.tts.stop()
  jarvis.out(T("Чтение остановлено", "Reading stopped"))
  return
end

if id == "assistant_tts_off" then
  jarvis.tts.set_enabled(false)
  say(T("Озвучка", "Speech"), T("Выключена", "Off"), false)
  return
end

if id == "assistant_tts_on" then
  jarvis.tts.set_enabled(true)
  say(T("Озвучка", "Speech"), T("Включена", "On"), T("Озвучка включена", "Speech is on"))
  return
end

local old
local text = ""

if id == "assistant_read_selection" then
  -- copy the selected text with Ctrl+C (sent to the program that has the focus), keep the old clipboard
  local ok_get, got = pcall(jarvis.system.clipboard.get)
  old = (ok_get and type(got) == "string") and got or nil
  local sent = jarvis.system.copy_selection()
  if sent then
    jarvis.sleep(350)
    ok_get, got = pcall(jarvis.system.clipboard.get)
    text = (ok_get and type(got) == "string") and got or ""
  end
  if text == "" or text == old then
    -- nothing new was copied: the Jarvis window had the focus (typed command) or nothing is selected
    text = old or ""
    if text ~= "" then
      jarvis.out(T("Выделение не получено, читаю текст из буфера обмена (Ctrl+C).", "No selection received, reading the clipboard text (Ctrl+C)."))
    end
  elseif old and old ~= "" then
    pcall(jarvis.system.clipboard.set, old)
  end
else
  local ok_get, got = pcall(jarvis.system.clipboard.get)
  text = (ok_get and type(got) == "string") and got or ""
end

text = text:gsub("^%s+", ""):gsub("%s+$", "")
if text == "" then
  say(T("Озвучка", "Speech"),
      T("Нет текста. Выделите текст, скопируйте (Ctrl+C) и скажите «прочитай буфер». Голосом «озвучь» работает прямо с выделенным.", "No text. Select text, copy it (Ctrl+C) and say \"read clipboard\". By voice, \"read this\" works with the selection directly."),
      T("Не вижу выделенного текста", "I don't see any selected text"))
  return
end

-- the preview is cut by characters (jarvis.text.preview): the old cut with string.sub counted bytes and split a Russian letter
-- in two, the broken string made jarvis.reply fail with "invalid UTF-8" and nothing was read aloud
-- nothing is reported as "reading" when the voice cannot speak (speech off, no Piper voice): the reason is said instead
local can, why = jarvis.tts.can_speak()
if not can then
  say(T("Озвучка", "Speech"),
      T("Не могу прочитать вслух: ", "Cannot read aloud: ") .. tostring(why) .. T(". Включите озвучку или установите голос в Настройках → Голос.", ". Turn speech on or install the voice in Settings → Voice."),
      false)
  return
end
say(T("Читаю", "Reading"), jarvis.text.preview(text, 80), false)
-- the clipboard text goes only to the local voice (Piper), never to an AI or another service
jarvis.speak(text)
