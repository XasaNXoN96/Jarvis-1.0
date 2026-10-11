-- weather (wttr.in): "какая погода", "погода в Ташкенте", "какая погода в Москве завтра"

local STOP = {
  "какая", "какой", "каков", "погода", "погоду", "погоды", "в", "во", "на", "сейчас", "сегодня", "завтра", "пожалуйста", "мне", "скажи", "покажи",
  "what", "is", "the", "weather", "in", "like", "today", "now", "please", "tell", "me", "how",
}

local function urlencode(s)
  return (s:gsub("[^%w%-_%.~]", function(c) return string.format("%%%02X", c:byte()) end))
end

local city = strip_words(phrase, STOP)
if city == "" then
  local chosen = jarvis.app.city and jarvis.app.city() or ""
  city = (chosen ~= "" and chosen) or jarvis.state.get("city") or "Tashkent"
end

local url = "https://wttr.in/" .. urlencode(city) .. "?format=3&lang=" .. (jarvis.context.language == "ua" and "uk" or jarvis.context.language)
jarvis.log("info", "Weather for " .. city .. ": " .. url)

local response = jarvis.http.get(url)
if response.ok then
  local body = (response.body or ""):gsub("%s+$", "")
  say(T("Погода", "Weather"), body, body)
else
  say(T("Погода", "Weather"), T("Не удалось получить погоду (нет интернета?)", "Could not get the weather (no internet?)"))
end

return { chain = false }
