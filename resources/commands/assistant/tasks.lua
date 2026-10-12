-- tasks: "добавь задачу купить хлеб", "озвучь задачи", "открой задачи", "выполнил задачу 2", "стереть все задачи"
-- One task per line in Documents\Jarvis-tasks.txt (the user can edit the file by hand too).

local id = jarvis.context.command_id
local path = docs_path("Jarvis-tasks.txt")
local TITLE = T("Задачи", "Tasks")

-- nil when the file exists but cannot be read: then nothing is written over it
local function read_tasks()
  local lines = read_lines(path)
  if not lines then return nil end
  local list = {}
  for _, raw in ipairs(lines) do
    local line = raw:gsub("^%s+", ""):gsub("%s+$", "")
    if line ~= "" then list[#list + 1] = line end
  end
  return list
end

local unreadable = T("Не удалось прочитать файл задач (он открыт в другой программе или нет доступа): ", "Cannot read the tasks file: ") .. path

local function write_tasks(list)
  local lines = {}
  for _, t in ipairs(list) do lines[#lines + 1] = t .. "\n" end
  return write_text(path, table.concat(lines))
end

if id == "assistant_task_open" then
  if not jarvis.fs.exists(path) then write_text(path, "") end
  open(path)
  say(TITLE, path, T("Открываю задачи", "Opening your tasks"))
  return
end

if id == "assistant_task_read" then
  local list = read_tasks()
  if not list then say(TITLE, unreadable); return end
  if #list == 0 then
    say(TITLE, T("Задач нет", "No tasks"))
    return
  end
  local shown, spoken = {}, {}
  for i, t in ipairs(list) do
    shown[#shown + 1] = i .. ". " .. t
    if i <= 7 then spoken[#spoken + 1] = i .. ". " .. t end
  end
  jarvis.reply(table.concat(shown, "\n"), T("Задачи", "Tasks") .. " (" .. #list .. ")")
  pcall(jarvis.speak, T("Задач: ", "Tasks: ") .. #list .. ". " .. table.concat(spoken, ". "))
  return
end

if id == "assistant_task_done" then
  local list = read_tasks()
  if not list then say(TITLE, unreadable); return end
  local n = jarvis.parse.number(phrase)
  if not n or n < 1 or n > #list then
    say(TITLE, T("Скажите номер задачи, например: выполнил задачу 2. Задач сейчас: ", "Say the task number, e.g. task 2 done. Tasks now: ") .. #list)
    return
  end
  local done = table.remove(list, n)
  if not write_tasks(list) then
    say(TITLE, T("Не удалось сохранить ", "Cannot save ") .. path)
    return
  end
  say(TITLE, T("Выполнено: ", "Done: ") .. done)
  return
end

if id == "assistant_task_clear" then
  local current = read_tasks()
  if not current then say(TITLE, unreadable); return end
  if #current == 0 then
    say(TITLE, T("Задач нет", "No tasks"))
    return
  end
  -- deleted only after "да" (office.lua, confirm_session)
  jarvis.session.start("confirm", 30)
  jarvis.session.set_data("tasks_clear")
  say(TITLE, T("Стереть все задачи? Скажите «да» или «нет».", "Delete all tasks? Say yes or no."))
  return
end

-- add a task
local STOP = { "добавь", "добавить", "новая", "новую", "задачу", "задача", "в", "список", "задач", "мне", "пожалуйста",
               "add", "a", "new", "task", "to", "the", "list", "please" }
local text = strip_leading(phrase, STOP)
if text == "" then
  say(TITLE, T("Какую задачу добавить? Например: добавь задачу купить хлеб", "Which task? For example: add task buy bread"))
  return
end
local list = read_tasks()
if not list then say(TITLE, unreadable); return end
-- the same limit as the panel "Задачи" of the window
if #list >= 30 then
  say(TITLE, T("Список полон: 30 задач. Удалите выполненные, например: выполнил задачу 1", "The list is full: 30 tasks. Remove done ones, e.g. task 1 done"))
  return
end
list[#list + 1] = text
if not write_tasks(list) then
  say(TITLE, T("Не удалось записать в ", "Cannot write to ") .. path)
  return
end
say(TITLE, T("Добавлена задача ", "Task added: ") .. #list .. ": " .. text, T("Задача добавлена", "Task added"))
