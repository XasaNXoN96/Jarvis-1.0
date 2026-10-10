-- brightness: "яркость 70 процентов", "ярче", "темнее"

local id = jarvis.context.command_id

local function apply(action, value)
  local extra = { "-Action", action }
  if value then extra[#extra + 1] = "-Value"; extra[#extra + 1] = tostring(value) end
  local r = ps_file("brightness.ps1", extra)
  if r.code == 2 then
    say(T("Яркость", "Brightness"), T(
      "Этот монитор не поддерживает регулировку яркости из Windows (работает на ноутбуках и встроенных экранах)",
      "This display does not support brightness control from Windows (laptops / built-in screens only)"))
    return nil
  end
  if not r.success then
    say(T("Яркость", "Brightness"), T("Не удалось изменить яркость", "Could not change the brightness"))
    jarvis.log("error", "brightness.ps1: " .. (r.stderr or ""))
    return nil
  end
  return tonumber((r.stdout or ""):match("(%d+)"))
end

if id == "assistant_brightness_set" then
  local n = jarvis.parse.number(phrase)
  if not n then
    say(T("Яркость", "Brightness"), T("Скажите уровень, например: яркость 70 процентов", "Say a level, e.g. brightness 70 percent"))
    return
  end
  n = math.max(0, math.min(100, math.floor(n + 0.5)))
  local now = apply("set", n)
  if now then say(T("Яркость", "Brightness"), now .. "%", T("Яркость " .. now .. " процентов", "Brightness " .. now .. " percent")) end
else
  local step = jarvis.parse.number(phrase) or 20
  step = math.max(1, math.min(100, math.floor(step + 0.5)))
  local now = apply(id == "assistant_brightness_up" and "up" or "down", step)
  if now then say(T("Яркость", "Brightness"), now .. "%", T("Яркость " .. now .. " процентов", "Brightness " .. now .. " percent")) end
end
