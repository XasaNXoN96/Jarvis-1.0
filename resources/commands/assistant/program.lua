-- open or close a program: "открой блокнот", "запусти дискорд", "закрой ворд", "открой тотал командер".
-- The names (and how they sound when heard: "ворт", "эксел", "тотал командер") live in crates/jarvis-core/src/apps.rs;
-- jarvis.apps.resolve() finds the program and the place where it is installed.

local closing = jarvis.context.command_id == "assistant_program_close"

local found = jarvis.apps.resolve(phrase)

if not found then
  say(T("Программа", "Program"), T("Не знаю такую программу", "Unknown program"))
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
  if ok then
    -- remember the document program: "пиши" then knows where to write
    if found.office then jarvis.state.set("office_app", found.office); jarvis.state.set("dict_last", "") end
    say(T("Открываю", "Opening"), found.name, T("Открываю " .. found.name, "Opening " .. found.name), "program")
  else
    jarvis.app.forget(key)
    say(T("Программа", "Program"), found.name .. T(": не удалось запустить. Скорее всего, она не установлена на этом компьютере", ": could not start it, probably it is not installed"))
  end
end
