-- computer info: "сколько оперативной памяти занято", "загрузка процессора", "свободное место на диске",
--                "какой у меня IP", "сколько работает компьютер"

local id = jarvis.context.command_id

local function ps(script)
  local r = powershell(script)
  return (r.stdout or ""):gsub("%s+$", "")
end

if id == "assistant_sys_ram" then
  local out = ps("$o = Get-CimInstance Win32_OperatingSystem; '{0}|{1}' -f [math]::Round(($o.TotalVisibleMemorySize - $o.FreePhysicalMemory)/1MB, 1), [math]::Round($o.TotalVisibleMemorySize/1MB, 1)")
  local used, total = out:match("([%d%.,]+)|([%d%.,]+)")
  if not used then say(T("Память", "Memory"), T("Не удалось узнать", "Could not read")); return end
  say(T("Память", "Memory"), used .. " / " .. total .. " GB", T("Занято " .. used .. " гигабайт из " .. total, "Using " .. used .. " of " .. total .. " gigabytes"))
elseif id == "assistant_sys_cpu" then
  local out = ps("[int](Get-CimInstance Win32_Processor | Measure-Object -Property LoadPercentage -Average).Average")
  local n = tonumber(out:match("(%d+)"))
  if not n then say(T("Процессор", "CPU"), T("Не удалось узнать", "Could not read")); return end
  say(T("Процессор", "CPU"), n .. "%", T("Процессор загружен на " .. n .. " процентов", "CPU load is " .. n .. " percent"))
elseif id == "assistant_sys_disk" then
  local out = ps("$d = Get-PSDrive C; '{0}|{1}' -f [int]($d.Free/1GB), [int](($d.Used + $d.Free)/1GB)")
  local free, total = out:match("(%d+)|(%d+)")
  if not free then say(T("Диск", "Disk"), T("Не удалось узнать", "Could not read")); return end
  say(T("Диск C", "Drive C"), free .. " / " .. total .. " GB", T("На диске си свободно " .. free .. " гигабайт из " .. total, "Drive C has " .. free .. " of " .. total .. " gigabytes free"))
elseif id == "assistant_sys_ip" then
  local out = ps("(Get-NetIPAddress -AddressFamily IPv4 | Where-Object { $_.InterfaceAlias -notmatch 'Loopback' -and $_.IPAddress -notlike '169.*' } | Select-Object -First 1).IPAddress")
  if out == "" then say(T("Сеть", "Network"), T("IP-адрес не найден (нет сети?)", "No IP address (no network?)")); return end
  say(T("IP-адрес", "IP address"), out, T("Ваш айпи адрес " .. out, "Your IP address is " .. out))
elseif id == "assistant_sys_uptime" then
  local out = ps("[int]((Get-Date) - (Get-CimInstance Win32_OperatingSystem).LastBootUpTime).TotalMinutes")
  local mins = tonumber(out:match("(%d+)"))
  if not mins then say(T("Компьютер", "Computer"), T("Не удалось узнать", "Could not read")); return end
  local text = fmt_duration(mins * 60)
  say(T("Компьютер работает", "Computer uptime"), text, T("Компьютер работает " .. text, "The computer has been running for " .. text))
end
