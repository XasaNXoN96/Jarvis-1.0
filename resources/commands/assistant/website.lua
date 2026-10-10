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
  say(T("Поиск", "Search"), rest, T("Ищу " .. rest, "Searching for " .. rest))
  return
end

-- open a website
local target = rest
if target == "" then
  open("https://www.google.com")
  say(T("Браузер", "Browser"), T("Открываю браузер", "Opening the browser"), nil, "website")
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
  say(T("Поиск", "Search"), target, T("Ищу " .. target, "Searching for " .. target))
  return
end

open("https://" .. domain)
say(T("Сайт", "Website"), domain, T("Открываю " .. domain, "Opening " .. domain), "website")
