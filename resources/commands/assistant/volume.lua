-- volume: "громкость 50 процентов", "громче", "тише", "выключи звук", "включи звук"

local id = jarvis.context.command_id
local ps1 = "volume.ps1"

local function apply(action, value)
  local extra = { "-Action", action }
  if value then extra[#extra + 1] = "-Value"; extra[#extra + 1] = tostring(value) end
  local r = ps_file(ps1, extra)
  if not r.success then
    say(T("Громкость", "Volume"), T("Не удалось изменить громкость", "Could not change the volume"))
    jarvis.log("error", "volume.ps1: " .. (r.stderr or ""))
    return nil
  end
  return tonumber((r.stdout or ""):match("(%d+)"))
end

if id == "assistant_volume_set" then
  local n = jarvis.parse.number(phrase)
  if not n then
    say(T("Громкость", "Volume"), T("Скажите уровень, например: громкость 50 процентов", "Say a level, e.g. volume 50 percent"))
    return
  end
  n = math.max(0, math.min(100, math.floor(n + 0.5)))
  apply("set", n)
  say(T("Громкость", "Volume"), n .. "%", T("Громкость " .. n .. " процентов", "Volume " .. n .. " percent"), n >= 90 and "volume_max" or nil)
elseif id == "assistant_volume_up" or id == "assistant_volume_down" then
  local step = jarvis.parse.number(phrase) or 10
  step = math.max(1, math.min(100, math.floor(step + 0.5)))
  local now = apply(id == "assistant_volume_up" and "up" or "down", step)
  say(T("Громкость", "Volume"), now and (now .. "%") or T("изменена", "changed"), now and T("Громкость " .. now .. " процентов", "Volume " .. now .. " percent") or T("Громкость изменена", "Volume changed"), (now and now >= 90) and "volume_max" or nil)
elseif id == "assistant_volume_mute" then
  apply("mute")
  say(T("Звук", "Sound"), T("Выключен", "Muted"), T("Звук выключен", "Sound off"), "volume_mute")
elseif id == "assistant_volume_unmute" then
  apply("unmute")
  say(T("Звук", "Sound"), T("Включён", "Unmuted"), T("Звук включён", "Sound on"))
end
