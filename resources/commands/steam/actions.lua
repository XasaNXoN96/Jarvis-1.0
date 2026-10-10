-- steam: open / close

if jarvis.context.command_id == "steam_1" then
  open("steam://open/main")
  say("Steam", T("Открываю Steam", "Opening Steam"))
else
  local r = run("taskkill", { "/f", "/im", "steam.exe" })
  say("Steam", r.success and T("Закрыт", "Closed") or T("Steam не запущен", "Steam is not running"))
end
