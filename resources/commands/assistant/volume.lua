-- volume: "громкость 50 процентов", "громче", "тише", "выключи звук", "включи звук"

local id = jarvis.context.command_id
local ps1 = "volume.ps1"

-- native Core Audio first; PowerShell script only as a fallback. Returns the level or nil.
local last_error = nil
local function apply(action, value)
  if WIN then
    local level, err = jarvis.system.volume(action, value or 0)
    if level then return level end
    last_error = err
    jarvis.log("error", "native volume: " .. tostring(err))
  end
  local extra = { "-Action", action }
  if value then extra[#extra + 1] = "-Value"; extra[#extra + 1] = tostring(value) end
  local r = ps_file(ps1, extra)
  if not r.success then
    last_error = jarvis.text.preview(r.stderr or "", 200)
    jarvis.log("error", "volume.ps1: " .. (r.stderr or ""))
    say(T("Громкость", "Volume"),
        T("Не удалось изменить громкость: ", "Could not change the volume: ") .. tostring(last_error),
        T("Не удалось изменить громкость", "Could not change the volume"))
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
  if not apply("set", n) then return end
  say(T("Громкость", "Volume"), n .. "%", T("Громкость " .. n .. " процентов", "Volume " .. n .. " percent"), n >= 90 and "volume_max" or nil)
elseif id == "assistant_volume_up" or id == "assistant_volume_down" then
  local step = jarvis.parse.number(phrase) or 10
  step = math.max(1, math.min(100, math.floor(step + 0.5)))
  local now = apply(id == "assistant_volume_up" and "up" or "down", step)
  if not now then return end
  say(T("Громкость", "Volume"), now and (now .. "%") or T("изменена", "changed"), now and T("Громкость " .. now .. " процентов", "Volume " .. now .. " percent") or T("Громкость изменена", "Volume changed"), (now and now >= 90) and "volume_max" or nil)
elseif id == "assistant_volume_mute" then
  if not apply("mute") then return end
  say(T("Звук", "Sound"), T("Выключен", "Muted"), T("Звук выключен", "Sound off"), "volume_mute")
elseif id == "assistant_volume_unmute" then
  if not apply("unmute") then return end
  say(T("Звук", "Sound"), T("Включён", "Unmuted"), T("Звук включён", "Sound on"))
end
