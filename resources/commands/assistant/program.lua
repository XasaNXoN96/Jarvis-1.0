-- open or close a program: "открой блокнот", "запусти дискорд", "закрой ворд"
-- To add a program: add a line to APPS (and its words to `triggers` in command.toml).

local APPS = {
  { stems = { "блокнот", "notepad" }, start = "notepad.exe", process = "notepad.exe", name = "Блокнот" },
  { stems = { "калькулятор", "calculator" }, start = "calc.exe", process = "CalculatorApp.exe", name = "Калькулятор" },
  { stems = { "пейнт", "paint" }, start = "mspaint.exe", process = "mspaint.exe", name = "Paint" },
  { stems = { "проводник", "explorer" }, start = "explorer.exe", process = nil, name = "Проводник" },
  { stems = { "диспетчер", "taskmgr", "task manager" }, start = "taskmgr.exe", process = "taskmgr.exe", name = "Диспетчер задач" },
  { stems = { "командную", "командная", "cmd", "терминал", "terminal" }, start = "cmd.exe", process = nil, name = "Командная строка" },
  { stems = { "powershell", "паувершелл", "пауэршелл" }, start = "powershell.exe", process = nil, name = "PowerShell" },
  { stems = { "настройки", "параметры", "settings" }, start = "ms-settings:", process = nil, name = "Настройки" },
  { stems = { "панель управления", "панель", "control panel" }, start = "control.exe", process = nil, name = "Панель управления" },
  { stems = { "ворд", "word" }, start = "winword.exe", process = "WINWORD.EXE", name = "Word" },
  { stems = { "эксель", "excel" }, start = "excel.exe", process = "EXCEL.EXE", name = "Excel" },
  { stems = { "пауэрпоинт", "powerpoint" }, start = "powerpnt.exe", process = "POWERPNT.EXE", name = "PowerPoint" },
  { stems = { "хром", "chrome" }, start = "chrome.exe", process = "chrome.exe", name = "Chrome" },
  { stems = { "эдж", "edge" }, start = "msedge.exe", process = "msedge.exe", name = "Edge" },
  { stems = { "фаерфокс", "firefox" }, start = "firefox.exe", process = "firefox.exe", name = "Firefox" },
  { stems = { "дискорд", "discord" }, start = "discord://", process = "Discord.exe", name = "Discord" },
  { stems = { "телеграм", "telegram" }, start = "tg://", process = "Telegram.exe", name = "Telegram" },
  { stems = { "спотифай", "spotify" }, start = "spotify:", process = "Spotify.exe", name = "Spotify" },
  { stems = { "стим", "steam" }, start = "steam://open/main", process = "steam.exe", name = "Steam" },
  { stems = { "обс", "obs" }, start = "obs64.exe", process = "obs64.exe", name = "OBS" },
  { stems = { "влс", "vlc" }, start = "vlc.exe", process = "vlc.exe", name = "VLC" },
}

local closing = jarvis.context.command_id == "assistant_program_close"

local function matches(app)
  for _, stem in ipairs(app.stems) do
    if stem:find(" ") then
      if phrase:find(stem, 1, true) then return true end
    elseif has_stem(stem) then
      return true
    end
  end
  return false
end

local found
for _, app in ipairs(APPS) do
  if matches(app) then found = app; break end
end

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
  local ok = pcall(open, found.start)
  if ok then
    -- remember the document program: "пиши" then knows where to write
    local OFFICE = { ["Word"] = "word", ["Excel"] = "excel", ["Блокнот"] = "notepad" }
    if OFFICE[found.name] then jarvis.state.set("office_app", OFFICE[found.name]); jarvis.state.set("dict_last", "") end
    say(T("Открываю", "Opening"), found.name, T("Открываю " .. found.name, "Opening " .. found.name), "program")
  else
    say(T("Программа", "Program"), T("Не удалось запустить ", "Could not start ") .. found.name .. T(" (возможно, не установлена)", " (maybe it is not installed)"))
  end
end
