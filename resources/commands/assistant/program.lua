-- open or close a program: "открой блокнот", "запусти дискорд", "закрой ворд", "открой тотал командер".
-- The names (and how they sound when heard: "ворт", "эксел", "тотал командер") live in crates/jarvis-core/src/apps.rs;
-- jarvis.apps.resolve() finds the program and the place where it is installed.

local closing = jarvis.context.command_id == "assistant_program_close" or jarvis.context.command_id == "assistant_program_force_close"

if jarvis.context.command_id == "assistant_apps_rescan" then
  say(T("Программы", "Programs"), T("Обновляю список установленных программ…", "Updating the list of installed programs…"), false)
  local n = jarvis.apps.rescan()
  say(T("Программы", "Programs"), T("Найдено программ: " .. n, "Programs found: " .. n), T("Список программ обновлён", "The list of programs is updated"))
  return
end

local found = jarvis.apps.resolve(phrase)

if not found then
  say(T("Программа", "Program"), T("Не нашёл такую программу на этом компьютере. Если она установлена недавно, скажите «обнови список программ».",
      "No such program on this computer. If it was installed recently, say \"update the list of programs\"."),
      T("Не нашёл такую программу", "I did not find this program"))
  return
end

-- "закрой" works only for the programs of the main list (their process name is known)
if closing and (found.ambiguous or found.indexed) then
  say(T("Программа", "Program"), T("Закрыть голосом можно только программы из основного списка (Word, Chrome, Telegram и другие). Эту программу закройте сами.",
      "Only programs of the main list can be closed by voice."))
  return
end

-- a name that fits several installed programs ("командер"): never a random one, the user chooses
if found.ambiguous then
  local lines = {}
  for i, name in ipairs(found.ambiguous) do lines[#lines + 1] = i .. ". " .. name end
  jarvis.session.start("app_choice", 30)
  say(T("Какую программу открыть?", "Which program?"), table.concat(lines, "\n") .. T("\nСкажите номер или название.", "\nSay the number or the name."),
      T("Нашёл несколько программ. Скажите номер: " , "Several programs fit. Say the number: ") .. table.concat(found.ambiguous, ", "))
  return
end

-- an installed program from the index (Start menu, desktop, registered apps)
if found.indexed then
  local key = "open:" .. found.name
  if not jarvis.app.once(key, 4) then
    say(T("Программа", "Program"), found.name .. T(" уже запускается", " is already starting"))
    return
  end
  local name, err, exe, was_running = jarvis.apps.launch_pending("1")
  if name then
    say(T("Открываю", "Opening"), name, T("Открываю " .. name, "Opening " .. name), "program")
    confirm_started(name, exe, key, was_running)
  else
    jarvis.app.forget(key)
    say(T("Программа", "Program"), T("Не удалось запустить: ", "Could not start: ") .. tostring(err))
  end
  return
end

if closing then
  if not found.process then
    say(T("Программа", "Program"), T("Эту программу нельзя закрыть голосом", "This program cannot be closed by voice") .. ": " .. found.name)
    return
  end
  -- the program is asked to close, like its own cross: Word, Notepad and the others ask about unsaved work.
  -- Killing it (/f) loses that work, so only "закрой … принудительно" does it, and that one asks "да / нет" first.
  -- (the words "принудительно" / "force" make the phrase belong to the other command, assistant_program_force_close, which asks first)
  local force = jarvis.context.command_id == "assistant_program_force_close"
  -- the names the program may run as: the known one, the file of its installed copy, and the 32/64-bit twin of each
  -- (Total Commander is TOTALCMD64.EXE or TOTALCMD.EXE depending on the installed copy); the ones that really run
  local names, seen = {}, {}
  local function add(n)
    if n and n ~= "" and not seen[n:lower()] then seen[n:lower()] = true; names[#names + 1] = n end
  end
  for _, n in ipairs({ found.process, found.path and exe_of(found.path) or nil }) do
    if n then
      add(n)
      local low = n:lower()
      if low:match("64%.exe$") then add((low:gsub("64%.exe$", ".exe"))) else add((low:gsub("%.exe$", "")) .. "64.exe") end
    end
  end
  local targets = { found.process }
  if jarvis.system.can_check_processes() then
    targets = {}
    for _, n in ipairs(names) do
      if jarvis.system.process_running(n) then targets[#targets + 1] = n end
    end
    if #targets == 0 then
      say(T("Программа", "Program"), found.name .. T(" не запущена", " is not running"))
      return
    end
  end
  local closed, last_code = false, 0
  for _, n in ipairs(targets) do
    local r = run("taskkill", force and { "/f", "/im", n } or { "/im", n })
    if r.success then closed = true else last_code = r.code end
  end
  local r = { success = closed, code = last_code }
  if r.success then
    if force then
      say(T("Закрыто", "Closed"), found.name .. T(" (принудительно)", " (forced)"), T("Закрываю " .. found.name, "Closing " .. found.name))
    else
      say(T("Закрываю", "Closing"), found.name .. T(". Если есть несохранённые изменения, программа спросит, сохранить ли их.", ". If there are unsaved changes, the program asks whether to save them."),
          T("Закрываю " .. found.name, "Closing " .. found.name))
    end
  elseif r.code == 128 then
    -- taskkill: no such process
    say(T("Программа", "Program"), found.name .. T(" не запущена", " is not running"))
  else
    say(T("Программа", "Program"), found.name .. T(" не закрылась сама (возможно, ждёт ответа в своём окне). Чтобы закрыть без сохранения, скажите «закрой " .. found.name .. " принудительно».",
        " did not close by itself (it may be waiting for an answer in its window). To close it without saving, say \"close " .. found.name .. " force\"."),
        T(found.name .. " не закрылась сама", found.name .. " did not close by itself"))
  end
else
  -- "открой ворд" heard or pressed twice in a row: one window, not two
  local key = "open:" .. found.name
  if not jarvis.app.once(key, 4) then
    say(T("Программа", "Program"), found.name .. T(" уже запускается", " is already starting"))
    return
  end
  -- the installed copy first (it is found even when the program is not in PATH), then the short name
  -- the process to wait for: the file of the installed copy when it is started directly, else the known process
  -- name of the program (calc.exe → CalculatorApp.exe); nothing when neither is known
  local exe_path = found.path and exe_of(found.path) or nil
  local was_running = running_before(exe_path) or running_before(found.process)
  local ok, launched = false, nil
  if found.path then ok = pcall(open, found.path); launched = found.path end
  if not ok then ok = pcall(open, found.start); launched = found.start end
  if not ok and found.web then
    -- the program is not installed but it has a website (Discord, Telegram, Spotify): the website opens instead
    if pcall(open, found.web) then
      jarvis.app.forget(key)
      say(T("Открываю сайт", "Opening the website"), found.name .. T(" (программа не установлена)", " (the program is not installed)"),
          T(found.name .. " не установлен, открываю сайт", found.name .. " is not installed, opening the website"), "website")
      return
    end
  end
  if ok then
    -- remember the document program: "пиши" then knows where to write
    if found.office then jarvis.state.set("office_app", found.office); jarvis.state.set("dict_last", "") end
    say(T("Открываю", "Opening"), found.name, T("Открываю " .. found.name, "Opening " .. found.name), "program")
    local expected = (launched == found.path and exe_path) or found.process
    local started = confirm_started(found.name, expected, key, was_running)
    -- the program that works with objects is open now: the engine knows it (objects.rs); a refusal (a task is running) is fine
    if started and (found.id == "vscode" or found.id == "word" or found.id == "excel" or found.id == "notepad" or found.id == "powerpoint") then
      jarvis.object.open_app(found.id)
    end
  else
    jarvis.app.forget(key)
    say(T("Программа", "Program"), found.name .. T(": не удалось запустить. Скорее всего, она не установлена на этом компьютере", ": could not start it, probably it is not installed"))
  end
end
