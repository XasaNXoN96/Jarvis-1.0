-- windows tools

local id = jarvis.context.command_id
local title = "Windows"

if id == "windows_1" then
  -- minimize all windows
  powershell("(New-Object -ComObject Shell.Application).MinimizeAll()")
  say(title, T("Окна свёрнуты", "Windows minimized"))
elseif id == "windows_2" then
  -- deleted for good: only after "да" (assistant/office.lua, confirm_session)
  jarvis.session.start("confirm", 30)
  jarvis.session.set_data("trash")
  say(title, T("Очистить корзину? Файлы удалятся навсегда. Скажите «да» или «нет».", "Empty the recycle bin? The files are deleted for good. Say yes or no."),
    T("Очистить корзину? Да или нет?", "Empty the recycle bin? Yes or no?"))
elseif id == "windows_3" then
  open("taskmgr.exe")
  say(title, T("Диспетчер задач", "Task Manager"))
elseif id == "windows_4" then
  open("ms-screenclip:")
  say(title, T("Снимок экрана", "Screenshot"))
elseif id == "windows_5" then
  say(title, T("Компьютер заблокирован", "Computer locked"), nil, "lock")
  run("rundll32.exe", { "user32.dll,LockWorkStation" })
elseif id == "windows_6" then
  say(title, T("Спящий режим", "Sleeping"))
  run("rundll32.exe", { "powrprof.dll,SetSuspendState", "0,1,0" })
elseif id == "windows_9" then
  -- the recording of Windows itself (Xbox Game Bar): Win+Alt+R starts and stops it; the video goes to Videos\Captures.
  -- Jarvis cannot see whether Game Bar is on, so it says what it did and where to look
  local ok, why = jarvis.system.keys("win+alt+r")
  if ok then
    say(title, T("Нажал Win+Alt+R — запись экрана Windows (Xbox Game Bar) начнётся или остановится. Видео сохраняется в «Видео\\Клипы» (Captures). Если ничего не произошло, включите Game Bar: Параметры → Игры → Xbox Game Bar.",
                 "Pressed Win+Alt+R: the Windows screen recording (Xbox Game Bar) starts or stops. Videos go to Videos\\Captures. If nothing happened, turn on Game Bar in Settings → Gaming."),
      T("Запись экрана", "Screen recording"))
  else
    say(title, T("Не удалось начать запись экрана: ", "Cannot start the screen recording: ") .. tostring(why))
  end
elseif id == "windows_8" then
  -- Alt+Shift switches the keyboard layout
  powershell("Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('%+')")
  say(title, T("Раскладка переключена", "Keyboard layout switched"))
end
