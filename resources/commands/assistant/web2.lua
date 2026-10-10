-- translate / wikipedia / maps / music search: opens the right page in the browser

local id = jarvis.context.command_id

local function urlencode(s)
  return (s:gsub("[^%w%-_%.~]", function(c) return string.format("%%%02X", c:byte()) end))
end

local LANGS = {
  ["английск"] = "en", ["русск"] = "ru", ["узбекск"] = "uz", ["турецк"] = "tr", ["китайск"] = "zh-CN", ["немецк"] = "de",
  ["французск"] = "fr", ["испанск"] = "es", ["украинск"] = "uk", ["арабск"] = "ar", ["итальянск"] = "it", ["японск"] = "ja",
  ["корейск"] = "ko", ["казахск"] = "kk", ["english"] = "en", ["russian"] = "ru", ["uzbek"] = "uz", ["turkish"] = "tr",
  ["chinese"] = "zh-CN", ["german"] = "de", ["french"] = "fr", ["spanish"] = "es", ["ukrainian"] = "uk", ["arabic"] = "ar",
}

if id == "assistant_translate" then
  local target = RU and "en" or "ru"
  local rest = {}
  for w in phrase:gmatch("%S+") do
    local bare = (w:gsub("[%.,!%?;:]+$", ""))
    local lang
    for stem, code in pairs(LANGS) do
      if bare:sub(1, #stem) == stem then lang = code end
    end
    if lang then target = lang
    elseif not ({ ["переведи"]=1, ["перевод"]=1, ["переведите"]=1, ["на"]=1, ["язык"]=1, ["языка"]=1, ["translate"]=1, ["to"]=1, ["into"]=1, ["in"]=1, ["пожалуйста"]=1, ["как"]=1, ["будет"]=1, ["сказать"]=1 })[bare] then
      rest[#rest + 1] = w
    end
  end
  local text = table.concat(rest, " ")
  if text == "" then
    say(T("Перевод", "Translate"), T("Что перевести? Например: переведи на английский привет", "What to translate?"))
    return
  end
  open("https://translate.google.com/?sl=auto&tl=" .. target .. "&text=" .. urlencode(text) .. "&op=translate")
  say(T("Перевод", "Translate"), text .. " → " .. target, T("Открываю переводчик", "Opening the translator"))

elseif id == "assistant_wiki" then
  local q = strip_words(phrase, { "найди", "найти", "в", "на", "во", "википедии", "википедия", "википедию", "что", "такое", "кто", "такой", "расскажи", "про", "о", "об", "wikipedia", "search", "find", "on", "in", "what", "is", "who", "tell", "me", "about", "the" })
  if q == "" then say(T("Википедия", "Wikipedia"), T("Что искать?", "What to look for?")); return end
  open("https://" .. (RU and "ru" or "en") .. ".wikipedia.org/w/index.php?search=" .. urlencode(q))
  say(T("Википедия", "Wikipedia"), q, T("Ищу в википедии " .. q, "Searching Wikipedia for " .. q))

elseif id == "assistant_maps" then
  local q = strip_words(phrase, { "покажи", "найди", "на", "карте", "карты", "карту", "где", "находится", "как", "добраться", "доехать", "до", "в", "маршрут", "построй", "show", "on", "the", "map", "where", "is", "route", "to", "directions", "how", "get", "find" })
  if q == "" then say(T("Карты", "Maps"), T("Что показать? Например: покажи на карте Ташкент", "What to show?")); return end
  local route = has_stem("добрат") or has_stem("доехат") or has_stem("маршрут") or has_stem("route") or has_stem("direction")
  if route then
    open("https://www.google.com/maps/dir/?api=1&destination=" .. urlencode(q))
  else
    open("https://www.google.com/maps/search/" .. urlencode(q))
  end
  say(T("Карты", "Maps"), q, route and T("Строю маршрут до " .. q, "Route to " .. q) or T("Показываю на карте " .. q, "Showing " .. q .. " on the map"))

elseif id == "assistant_music_search" then
  local q = strip_words(phrase, { "включи", "поставь", "запусти", "найди", "песню", "песня", "клип", "трек", "музыку", "видео", "в", "ютубе", "ютуб", "youtube", "play", "put", "on", "song", "music", "video", "clip", "track", "me", "some" })
  if q == "" then say(T("Музыка", "Music"), T("Что включить? Например: включи песню Imagine Dragons", "What to play?")); return end
  open("https://www.youtube.com/results?search_query=" .. urlencode(q))
  say(T("Ютуб", "YouTube"), q, T("Ищу на ютубе " .. q, "Searching YouTube for " .. q), "website")
end
