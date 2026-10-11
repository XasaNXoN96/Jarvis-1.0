-- "переведи выделенное", "проверь скорость интернета", "переключи на оперу"

local id = jarvis.context.command_id

local function urlencode(s)
  return (s:gsub("[^%w%-_%.~ ]", function(c) return string.format("%%%02X", string.byte(c)) end):gsub(" ", "+"))
end

if id == "assistant_translate_clip" then
  -- the selected text (Ctrl+C in the active window) or the clipboard; it opens in Google Translate (a site, not the AI)
  local ok_get, got = pcall(jarvis.system.clipboard.get)
  local old = (ok_get and type(got) == "string") and got or ""
  local text = ""
  if jarvis.system.copy_selection() then
    jarvis.sleep(350)
    ok_get, got = pcall(jarvis.system.clipboard.get)
    text = (ok_get and type(got) == "string") and got or ""
  end
  if text == "" or text == old then text = old end
  text = text:gsub("^%s+", ""):gsub("%s+$", "")
  if text == "" then
    say(T("Перевод", "Translate"), T("Нет текста: выделите или скопируйте текст и повторите.", "No text: select or copy a text and try again."))
    return
  end
  local to = (phrase:find("англ") or phrase:find("english")) and "en" or "ru"
  open("https://translate.google.com/?sl=auto&tl=" .. to .. "&op=translate&text=" .. urlencode(jarvis.text.preview(text, 4000)))
  say(T("Перевод", "Translate"), jarvis.text.preview(text, 60), T("Открываю перевод", "Opening the translation"))
  return
end

if id == "assistant_speedtest" then
  open("https://www.speedtest.net/")
  say(T("Скорость интернета", "Internet speed"), "speedtest.net", T("Открываю проверку скорости", "Opening the speed test"))
  return
end

if id == "assistant_browser_set" then
  local BROWSERS = { { "опер", "opera", "Opera" }, { "эдж", "edge", "Edge" }, { "edge", "edge", "Edge" }, { "хром", "chrome", "Chrome" },
                     { "chrome", "chrome", "Chrome" }, { "фаерфокс", "firefox", "Firefox" }, { "firefox", "firefox", "Firefox" },
                     { "брейв", "brave", "Brave" }, { "яндекс", "yandex", "Яндекс" }, { "систем", "", T("системный", "system") } }
  local p = phrase:lower()
  for _, b in ipairs(BROWSERS) do
    if p:find(b[1], 1, true) then
      jarvis.app.set_browser(b[2])
      say(T("Браузер", "Browser"), T("Сайты будут открываться: ", "Sites will open in: ") .. b[3])
      return
    end
  end
  say(T("Браузер", "Browser"), T("Скажите, например: переключи на оперу, эдж, хром, фаерфокс, яндекс браузер", "Say e.g.: switch to opera, edge, chrome, firefox"))
  return
end
