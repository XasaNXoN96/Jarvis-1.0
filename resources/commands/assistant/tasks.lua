-- tasks: "добавь задачу купить хлеб", "озвучь задачи", "открой задачи", "выполнил задачу 2", "стереть все задачи"
-- One task per line in Documents\Jarvis-tasks.txt (the user can edit the file by hand too).

local id = jarvis.context.command_id
local home = jarvis.system.env("USERPROFILE") or jarvis.system.env("HOME") or "."
local path = home .. "/Documents/Jarvis-tasks.txt"
local TITLE = T("Задачи", "Tasks")

local function read_tasks()
  local list = {}
  local f = io.open(path, "r")
  if not f then return list end
  for raw in f:lines() do
    local line = raw:gsub("^%s+", ""):gsub("%s+$", "")
    if line ~= "" then list[#list + 1] = line end
  end
  f:close()
  return list
end

local function write_tasks(list)
  local f = io.open(path, "w")
  if not f then return false end
  for _, t in ipairs(list) do f:write(t .. "\n") end
  f:close()
  return true
end

if id == "assistant_task_open" then
  local f = io.open(path, "a")
  if f then f:close() end
  open(path)
  say(TITLE, path, T("Открываю задачи", "Opening your tasks"))
  return
end

if id == "assistant_task_read" then
  local list = read_tasks()
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
  if #read_tasks() == 0 then
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
list[#list + 1] = text
if not write_tasks(list) then
  say(TITLE, T("Не удалось записать в ", "Cannot write to ") .. path)
  return
end
say(TITLE, T("Добавлена задача ", "Task added: ") .. #list .. ": " .. text, T("Задача добавлена", "Task added"))
