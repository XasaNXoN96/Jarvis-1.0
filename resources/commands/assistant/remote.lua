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

-- the link holds the secret of the remote: it is shown only in the window (QR code), never written into the
-- conversation history or the clipboard (Windows keeps clipboard history and may sync it)
jarvis.app.show_page("settings")   -- the QR code is shown there
say(T("Пульт", "Remote"), T("Пульт включён. QR-код и ссылка — в окне Jarvis: Настройки → Пульт для телефона.",
    "Remote control is on. The QR code and the link are in the Jarvis window: Settings → Phone remote."),
    T("Пульт включён. Наведите камеру телефона на QR-код в настройках.", "Remote control is on. Point the phone camera at the QR code in Settings."))
