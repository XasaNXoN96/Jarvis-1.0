-- voice / text mode: "режим текста" (microphone off), "режим голоса" (microphone on)

if jarvis.context.command_id == "assistant_mic_off" then
  jarvis.app.set_mic_muted(true)
  say(T("Микрофон", "Microphone"), T("Выключен. Команды принимаются текстом (консоль, пульт). Включить: иконка в трее.",
      "Off. Commands are accepted as text (console, remote). Turn on: tray icon."),
      T("Микрофон выключен", "Microphone off"))
else
  jarvis.app.set_mic_muted(false)
  say(T("Микрофон", "Microphone"), T("Включён", "On"), T("Микрофон включён", "Microphone on"))
end
