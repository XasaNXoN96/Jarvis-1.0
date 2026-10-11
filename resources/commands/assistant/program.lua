-- open or close a program: "открой блокнот", "запусти дискорд", "закрой ворд", "открой тотал командер".
-- The names (and how they sound when heard: "ворт", "эксел", "тотал командер") live in crates/jarvis-core/src/apps.rs;
-- jarvis.apps.resolve() finds the program and the place where it is installed.

local closing = jarvis.context.command_id == "assistant_program_close"

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
  local name, err = jarvis.apps.launch_pending("1")
  if name then
    say(T("Открываю", "Opening"), name, T("Открываю " .. name, "Opening " .. name), "program")
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
  local r = run("taskkill", { "/f", "/im", found.process })
  if r.success then
    say(T("Закрыто", "Closed"), found.name, T("Закрываю " .. found.name, "Closing " .. found.name))
  else
    say(T("Программа", "Program"), found.name .. T(" не запущена", " is not running"))
  end
else
  -- "открой ворд" heard or pressed twice in a row: one window, not two
  local key = "open:" .. found.name
  if not jarvis.app.once(key, 4) then
    say(T("Программа", "Program"), found.name .. T(" уже запускается", " is already starting"))
    return
  end
  -- the installed copy first (it is found even when the program is not in PATH), then the short name
  local ok = false
  if found.path then ok = pcall(open, found.path) end
  if not ok then ok = pcall(open, found.start) end
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
  else
    jarvis.app.forget(key)
    say(T("Программа", "Program"), found.name .. T(": не удалось запустить. Скорее всего, она не установлена на этом компьютере", ": could not start it, probably it is not installed"))
  end
end
