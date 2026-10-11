-- "установи город Ташкент": the city used when you only ask "какая погода"

local city = strip_words(phrase, {
  "установи", "поставь", "смени", "измени", "мой", "город", "городом", "по", "умолчанию", "на", "пожалуйста",
  "set", "change", "my", "city", "default", "to", "please", "the",
})
if city == "" then
  say(T("Город", "City"), T("Скажите город, например: установи город Ташкент", "Say the city, e.g. set city Tashkent"))
  return
end
jarvis.state.set("city", city)
pcall(jarvis.app.set_city, city)   -- the weather block of the window uses the same city
say(T("Город", "City"), city, T("Запомнил город " .. city, "City saved: " .. city))
