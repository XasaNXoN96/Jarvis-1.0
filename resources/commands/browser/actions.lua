-- browser: open / close

local id = jarvis.context.command_id

if id == "browser_open" then
  open("https://www.google.com")
  say(T("Браузер", "Browser"), T("Открываю браузер", "Opening the browser"))
elseif id == "browser_close" then
  local closed = 0
  for _, exe in ipairs({ "chrome.exe", "msedge.exe", "firefox.exe", "browser.exe", "opera.exe", "brave.exe" }) do
    local r = run("taskkill", { "/f", "/im", exe })
    if r.success then closed = closed + 1 end
  end
  say(T("Браузер", "Browser"), closed > 0 and T("Закрыт", "Closed") or T("Браузер не запущен", "No browser is running"))
elseif id == "open_google" then
  open("https://www.google.com")
  say(T("Сайт", "Website"), "google.com")
end
