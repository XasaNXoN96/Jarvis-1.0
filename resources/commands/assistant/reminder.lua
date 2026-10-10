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


-- reminders: "напомни через 5 минут позвонить маме", "поставь напоминание на 10 минут",
--            "скажи мне через 5 минут, чтобы я закончил игру"

local secs, rest = jarvis.parse.duration(phrase)
if not secs then
  say(T("Напоминание", "Reminder"), T("Скажите время, например: напомни через 5 минут", "Say the time, e.g. remind me in 5 minutes"))
  return
end

local STOP = {
  "напомни", "напомнить", "напоминание", "напоминалку", "поставь", "поставить", "установи", "создай",
  "мне", "меня", "пожалуйста", "чтобы", "что", "бы", "я", "через", "на", "скажи", "сообщи", "предупреди",
  "remind", "me", "to", "that", "in", "set", "a", "reminder", "please", "tell", "warn",
}
local text = strip_words(rest or "", STOP)

local message
if text ~= "" then
  message = text
  if has_stem("чтобы") or has_stem("that") then
    -- "...чтобы я закончил игру": we cannot conjugate, so state it plainly
    message = T("Время вышло! Вы просили: ", "Time is up! You asked: ") .. text
  end
elseif has_stem("останов") or has_stem("заканч") or has_stem("прекрат") then
  message = T("Время вышло, пора заканчивать!", "Time is up, time to stop!")
else
  message = T("Напоминание!", "Reminder!")
end

jarvis.timer.after(secs, T("Напоминание", "Reminder"), message)
say(T("Напоминание", "Reminder"), T("Через " .. fmt_duration(secs) .. ": " .. message, "In " .. fmt_duration(secs) .. ": " .. message))
