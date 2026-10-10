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


-- "открой сайт ютуб", "зайди на сайт example.com", "найди в гугле погоду"

local SITES = {
  ["ютуб"] = "youtube.com", ["youtube"] = "youtube.com", ["ютьюб"] = "youtube.com",
  ["гугл"] = "google.com", ["google"] = "google.com",
  ["яндекс"] = "ya.ru", ["yandex"] = "ya.ru",
  ["вк"] = "vk.com", ["вконтакте"] = "vk.com", ["vk"] = "vk.com",
  ["телеграм"] = "web.telegram.org", ["telegram"] = "web.telegram.org",
  ["ватсап"] = "web.whatsapp.com", ["whatsapp"] = "web.whatsapp.com",
  ["фейсбук"] = "facebook.com", ["facebook"] = "facebook.com",
  ["инстаграм"] = "instagram.com", ["instagram"] = "instagram.com",
  ["твиттер"] = "x.com", ["twitter"] = "x.com",
  ["гитхаб"] = "github.com", ["github"] = "github.com",
  ["википедия"] = "wikipedia.org", ["википедию"] = "wikipedia.org", ["wikipedia"] = "wikipedia.org",
  ["почта"] = "mail.google.com", ["почту"] = "mail.google.com", ["gmail"] = "mail.google.com",
  ["хабр"] = "habr.com", ["твич"] = "twitch.tv", ["twitch"] = "twitch.tv",
  ["дискорд"] = "discord.com", ["discord"] = "discord.com",
  ["реддит"] = "reddit.com", ["reddit"] = "reddit.com",
  ["нетфликс"] = "netflix.com", ["netflix"] = "netflix.com",
  ["тикток"] = "tiktok.com", ["tiktok"] = "tiktok.com",
  ["озон"] = "ozon.ru", ["вайлдберриз"] = "wildberries.ru", ["авито"] = "avito.ru",
  ["алиэкспресс"] = "aliexpress.com", ["амазон"] = "amazon.com", ["amazon"] = "amazon.com",
  ["кинопоиск"] = "kinopoisk.ru", ["переводчик"] = "translate.google.com",
  ["карты"] = "maps.google.com", ["maps"] = "maps.google.com",
}

local STOP = {
  "открой", "открыть", "запусти", "зайди", "зайти", "перейди", "перейти", "включи", "покажи",
  "на", "в", "во", "сайт", "сайта", "страницу", "страница", "браузер", "браузере", "и", "мне", "пожалуйста",
  "найди", "найти", "поищи", "загугли", "гугле", "гугл", "интернете", "ютубе", "в", "по", "про",
  "open", "go", "to", "website", "site", "the", "visit", "launch", "browser", "and", "please",
  "search", "for", "find", "google", "on", "youtube", "in", "internet",
}

local function urlencode(s)
  return (s:gsub("[^%w%-_%.~]", function(c) return string.format("%%%02X", c:byte()) end))
end

local searching = has_stem("найд") or has_stem("найт") or has_stem("поищ") or has_stem("загугл")
  or has_stem("search") or has_stem("find")
local on_youtube = has_stem("ютуб") or has_stem("ютьюб") or has_stem("youtube")

-- drop command words only at the START of the phrase, the query itself stays intact
local function strip_leading(text, stop)
  local set = {}
  for _, w in ipairs(stop) do set[w] = true end
  local out, started = {}, false
  for w in text:gmatch("%S+") do
    local bare = (w:gsub("[%.,!%?;:]+$", ""))
    if started or not set[bare] then
      started = true
      out[#out + 1] = w
    end
  end
  return table.concat(out, " ")
end

local rest = strip_leading(phrase, STOP)

if searching then
  if rest == "" then
    say(T("Поиск", "Search"), T("Что искать? Например: найди в гугле погоду", "What to search for?"))
    return
  end
  local url
  if on_youtube then
    url = "https://www.youtube.com/results?search_query=" .. urlencode(rest)
  else
    url = "https://www.google.com/search?q=" .. urlencode(rest)
  end
  open(url)
  say(T("Поиск", "Search"), rest)
  return
end

-- open a website
local target = rest
if target == "" then
  open("https://www.google.com")
  say(T("Браузер", "Browser"), T("Открываю браузер", "Opening the browser"))
  return
end

-- spoken dots: "example точка com"
target = target:gsub("%s+точка%s+", "."):gsub("%s+dot%s+", ".")

local domain = SITES[target]
if not domain then
  -- first word may be a known site: "ютуб музыка"
  local first = target:match("^(%S+)")
  if first and SITES[first] and not target:find("%.") then domain = SITES[first] end
end
if not domain and target:find("%.") and not target:find("%s") then
  domain = target:gsub("^https?://", "")
end
if not domain then
  -- unknown name: search for it
  open("https://www.google.com/search?q=" .. urlencode(target))
  say(T("Поиск", "Search"), target)
  return
end

open("https://" .. domain)
say(T("Сайт", "Website"), domain)
