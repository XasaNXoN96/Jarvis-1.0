-- helpers shared by the assistant scripts
local WIN = jarvis.system.platform == "windows"
local RU = jarvis.context.language == "ru" or jarvis.context.language == "ua"
local function T(ru, en) if RU then return ru end return en end
local phrase = (jarvis.context.phrase or ""):lower()

-- run a program (no shell). On non-Windows systems only logs what would be done.
local function run(program, args)
  if not WIN then
    jarvis.log("info", "[not windows, skipped] " .. program .. " " .. table.concat(args or {}, " "))
    return { success = true, code = 0, stdout = "", stderr = "" }
  end
  return jarvis.system.run(program, args or {})
end

-- open a URL / file / program through the shell
local function open(target)
  if not WIN then
    jarvis.log("info", "[not windows, skipped] open " .. target)
    return true
  end
  return jarvis.system.open(target)
end

-- tell the user: console + notification
local function say(title, msg)
  jarvis.out(title .. ": " .. msg)
  jarvis.system.notify(title, msg)
end

local function words()
  local list = {}
  for w in phrase:gmatch("%S+") do
    list[#list + 1] = (w:gsub("[%.,!%?;:]+$", ""))
  end
  return list
end

-- true when some word of the phrase starts with `stem`
local function has_stem(stem)
  for _, w in ipairs(words()) do
    if w:sub(1, #stem) == stem then return true end
  end
  return false
end

local function plural_ru(n, one, few, many)
  local m100, m10 = n % 100, n % 10
  if m100 >= 11 and m100 <= 19 then return many end
  if m10 == 1 then return one end
  if m10 >= 2 and m10 <= 4 then return few end
  return many
end

-- 5400 -> "1 час 30 минут"
local function fmt_duration(secs)
  local h = secs // 3600
  local m = (secs % 3600) // 60
  local s = secs % 60
  local parts = {}
  if h > 0 then parts[#parts + 1] = h .. " " .. (RU and plural_ru(h, "час", "часа", "часов") or (h == 1 and "hour" or "hours")) end
  if m > 0 then parts[#parts + 1] = m .. " " .. (RU and plural_ru(m, "минуту", "минуты", "минут") or (m == 1 and "minute" or "minutes")) end
  if s > 0 and h == 0 then parts[#parts + 1] = s .. " " .. (RU and plural_ru(s, "секунду", "секунды", "секунд") or (s == 1 and "second" or "seconds")) end
  return table.concat(parts, " ")
end

-- remove stop words from a phrase, returns the remaining text
local function strip_words(text, stop)
  local set = {}
  for _, w in ipairs(stop) do set[w] = true end
  local out = {}
  for w in text:gmatch("%S+") do
    local bare = (w:gsub("[%.,!%?;:]+$", ""))
    if not set[bare] then out[#out + 1] = w end
  end
  return table.concat(out, " ")
end

local PS = { "-NoProfile", "-ExecutionPolicy", "Bypass" }
local function powershell(command)
  local args = { table.unpack(PS) }
  args[#args + 1] = "-Command"
  args[#args + 1] = command
  return run("powershell", args)
end
local function ps_file(name, extra)
  local args = { table.unpack(PS) }
  args[#args + 1] = "-File"
  args[#args + 1] = jarvis.context.command_path .. "/" .. name
  for _, a in ipairs(extra or {}) do args[#args + 1] = a end
  return run("powershell", args)
end


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
    say(T("Закрыто", "Closed"), found.name)
  else
    say(T("Программа", "Program"), found.name .. T(" не запущена", " is not running"))
  end
else
  local ok = open(found.start)
  if ok then
    say(T("Открываю", "Opening"), found.name)
  else
    say(T("Программа", "Program"), T("Не удалось запустить ", "Could not start ") .. found.name)
  end
end
