-- music / video: "пауза", "продолжи", "следующий трек", "предыдущий трек"

local id = jarvis.context.command_id
local key, ru, en
if id == "assistant_media_next" then
  key, ru, en = "next", "Следующий трек", "Next track"
elseif id == "assistant_media_prev" then
  key, ru, en = "prev", "Предыдущий трек", "Previous track"
else
  key, ru, en = "playpause", "Пауза или продолжение", "Play / pause"
end

local r = ps_file("media.ps1", { "-Key", key })
if r.success then
  say(T("Медиа", "Media"), T(ru, en), false)
else
  say(T("Медиа", "Media"), T("Не удалось отправить команду плееру", "Could not send the command to the player"))
end
