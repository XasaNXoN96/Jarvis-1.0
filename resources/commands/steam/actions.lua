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



-- steam: open / close

if jarvis.context.command_id == "steam_1" then
  open("steam://open/main")
  say("Steam", T("Открываю Steam", "Opening Steam"))
else
  local r = run("taskkill", { "/f", "/im", "steam.exe" })
  say("Steam", r.success and T("Закрыт", "Closed") or T("Steam не запущен", "Steam is not running"))
end
