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
  -- Killing it (/f) loses that work, so only "закрой … принудительно" does it.
  local force = has_stem("принудит") or has_stem("force")
  local r = run("taskkill", force and { "/f", "/im", found.process } or { "/im", found.process })
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
  local was_running = running_before(found.path) or running_before(found.start)
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
    confirm_started(found.name, launched, key, was_running)
  else
    jarvis.app.forget(key)
    say(T("Программа", "Program"), found.name .. T(": не удалось запустить. Скорее всего, она не установлена на этом компьютере", ": could not start it, probably it is not installed"))
  end
end
