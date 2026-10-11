-- phone remote: "включи удалённое управление", "выключи удалённое управление"

if jarvis.context.command_id == "assistant_remote_off" then
  jarvis.remote.disable()
  say(T("Пульт", "Remote"), T("Удалённое управление выключено", "Remote control is off"))
  return
end

local url, err = jarvis.remote.enable()
if not url then
  say(T("Пульт", "Remote"), T("Не удалось включить: ", "Could not enable: ") .. (err or "?"))
  return
end

jarvis.out(url)
jarvis.app.show_page("settings")   -- the QR code is shown there
if jarvis.system.clipboard.set then pcall(jarvis.system.clipboard.set, url) end
say(T("Пульт", "Remote"), T("Ссылка для телефона скопирована в буфер обмена: ", "Phone link copied to the clipboard: ") .. url,
    T("Пульт включён. Наведите камеру телефона на QR-код в настройках.", "Remote control is on. Point the phone camera at the QR code in Settings."))
