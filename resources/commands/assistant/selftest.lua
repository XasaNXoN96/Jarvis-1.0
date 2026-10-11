-- "диагностика": checks the important things and tells what does not work

local function line(ok, text) jarvis.out((ok and "  [ OK ] " or "  [FAIL] ") .. text) end

jarvis.out(T("=== Диагностика Jarvis ===", "=== Jarvis diagnostics ==="))
local problems = 0
local function check(ok, good, bad)
  line(ok, ok and good or bad)
  if not ok then problems = problems + 1 end
end

check(jarvis.system.platform == "windows", T("Windows", "Windows"), T("Не Windows: " .. jarvis.system.platform, "Not Windows: " .. jarvis.system.platform))
check(true, T("Язык: " .. jarvis.context.language, "Language: " .. jarvis.context.language), "")

-- PowerShell + the volume script (the same one the volume commands use)
local r = ps_file("volume.ps1", { "-Action", "get" })
local vol = tonumber((r.stdout or ""):match("(%d+)"))
check(r.success and vol ~= nil,
      T("PowerShell и громкость: сейчас " .. tostring(vol) .. "%", "PowerShell and volume: now " .. tostring(vol) .. "%"),
      T("Громкость не читается. PowerShell: " .. tostring((r.stderr or ""):sub(1, 200)), "Cannot read the volume: " .. tostring((r.stderr or ""):sub(1, 200))))

local b = ps_file("brightness.ps1", { "-Action", "get" })
if b.code == 2 then
  line(true, T("Яркость: монитор не поддерживает регулировку (это нормально для внешних мониторов)", "Brightness: this display has no software control (normal for external monitors)"))
else
  local br = tonumber((b.stdout or ""):match("(%d+)"))
  check(b.success and br ~= nil, T("Яркость: " .. tostring(br) .. "%", "Brightness: " .. tostring(br) .. "%"), T("Яркость не читается", "Cannot read the brightness"))
end

-- notifications and speech
check(pcall(jarvis.system.notify, "Jarvis", T("Проверка уведомлений", "Notification test")),
      T("Уведомление отправлено (видно ли оно в углу экрана?)", "Notification sent (do you see it in the corner?)"), T("Уведомления не работают", "Notifications failed"))
jarvis.out(T("  Озвучка включена: ", "  Speech enabled: ") .. tostring(jarvis.tts.is_enabled()))

-- timers
local before = #jarvis.timer.list()
local id = jarvis.timer.after(5, T("Диагностика", "Diagnostics"), T("Тестовый таймер сработал", "Test timer fired"))
check(#jarvis.timer.list() == before + 1, T("Таймеры работают (тест сработает через 5 секунд)", "Timers work (the test fires in 5 seconds)"), T("Таймер не создался", "The timer was not created"))

-- notes folder
local home = jarvis.system.env("USERPROFILE") or "."
local f = io.open(home .. "/Documents/Jarvis-notes.txt", "a")
check(f ~= nil, T("Папка для заметок доступна", "Notes folder is writable"), T("Нельзя писать в Документы", "Cannot write to Documents"))
if f then f:close() end

-- commands
local n = #jarvis.app.commands()
check(n > 20, T("Команд загружено: " .. n, "Commands loaded: " .. n), T("Команд мало: " .. n .. " (обновите папку resources\\commands)", "Few commands: " .. n))

-- the file search and window
jarvis.out(T("  Микрофон выключен: ", "  Microphone muted: ") .. tostring(jarvis.app.is_mic_muted()))

jarvis.out(problems == 0 and T("Итог: проблем не найдено.", "Result: no problems found.")
           or T("Итог: проблем найдено: " .. problems .. ". Пришлите этот список разработчику.", "Result: problems found: " .. problems))
jarvis.tts.diagnose()   -- voices list + spoken test
