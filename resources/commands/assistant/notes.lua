-- voice notes: "запиши идею купить молоко", "что я записывал", "открой заметки"

local id = jarvis.context.command_id
local path = docs_path("Jarvis-notes.txt")

if id == "assistant_note_open" then
  if not jarvis.fs.exists(path) then write_text(path, "") end
  open(path)
  say(T("Заметки", "Notes"), path, T("Открываю заметки", "Opening your notes"))
  return
end

if id == "assistant_note_read" then
  local lines = {}
  for _, line in ipairs(read_lines(path) or {}) do
    if line ~= "" then lines[#lines + 1] = line end
  end
  if #lines == 0 then
    say(T("Заметки", "Notes"), T("Заметок пока нет", "No notes yet"))
    return
  end
  local first = math.max(1, #lines - 4)
  local spoken = {}
  for i = first, #lines do
    jarvis.out("  " .. lines[i])
    -- spoken part: the note text without the date prefix "[2026-10-10 14:05] "
    spoken[#spoken + 1] = (lines[i]:gsub("^%[[^%]]*%]%s*", ""))
  end
  say(T("Заметки", "Notes"), T("Последние: " .. (#lines - first + 1), "Latest: " .. (#lines - first + 1)),
      T("Последние заметки. ", "Latest notes. ") .. table.concat(spoken, ". "))
  return
end

-- add a note
local STOP = {
  "запиши", "записать", "запомни", "запомнить", "добавь", "заметка", "заметку", "заметки", "идею", "идея",
  "мысль", "в", "что", "мне", "пожалуйста",
  "note", "write", "down", "remember", "save", "add", "a", "idea", "thought", "that", "please",
}
local text = strip_leading(phrase, STOP)
if text == "" then
  say(T("Заметки", "Notes"), T("Что записать? Например: запиши идею купить молоко", "What to write down?"))
  return
end

local line = string.format("[%s-%s-%s %s:%s] %s\n", jarvis.context.time.year, jarvis.context.time.month,
  jarvis.context.time.day, jarvis.context.time.hour, jarvis.context.time.minute, text)
if not append_text(path, line) then
  say(T("Заметки", "Notes"), T("Не удалось записать в ", "Cannot write to ") .. path)
  return
end
say(T("Заметка", "Note"), text, T("Записал", "Noted"), "note")
