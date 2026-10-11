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
elseif id == "windows_8" then
  -- Alt+Shift switches the keyboard layout
  powershell("Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.SendKeys]::SendWait('%+')")
  say(title, T("Раскладка переключена", "Keyboard layout switched"))
end
