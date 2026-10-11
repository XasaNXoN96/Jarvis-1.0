-- "открой сайт ютуб", "зайди на сайт example.com", "найди в гугле погоду"

-- the sites themselves live in ONE catalogue (crates/jarvis-core/src/sites.rs): jarvis.sites.direct / jarvis.sites.find

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

-- "открой YouTube": a known site by name is opened, never searched
if not searching then
  local url, name = jarvis.sites.direct(phrase)
  if url then
    open(url)
    say(T("Сайт", "Website"), name, T("Открываю " .. name, "Opening " .. name), "website")
    return
  end
end

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

local url, name = jarvis.sites.find(target)
if not url then
  -- the first words may be a known site: "ютуб музыка", "яндекс карты пробки"
  local w = {}
  for x in target:gmatch("%S+") do w[#w + 1] = x end
  for n = math.min(#w, 3), 1, -1 do
    url, name = jarvis.sites.find(table.concat(w, " ", 1, n))
    if url then break end
  end
end
if url then
  open(url)
  say(T("Сайт", "Website"), name, T("Открываю " .. name, "Opening " .. name), "website")
  return
end
local domain
if target:find("%.") and not target:find("%s") then
  domain = target:gsub("^https?://", "")
end
if not domain then
  -- an unknown name: said plainly that this is a search, not a site that was found
  open("https://www.google.com/search?q=" .. urlencode(target))
  say(T("Поиск", "Search"), T("Не знаю такой сайт, ищу в Google: ", "Unknown site, searching Google: ") .. target, T("Ищу " .. target, "Searching for " .. target))
  return
end

open("https://" .. domain)
say(T("Сайт", "Website"), domain, T("Открываю " .. domain, "Opening " .. domain), "website")
