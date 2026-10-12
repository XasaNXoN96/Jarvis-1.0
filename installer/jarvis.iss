; Inno Setup script. Run from the repository root: ISCC.exe installer\jarvis.iss
; The installer is written to the dist\ folder.
; Parameters: /DSrcDir=build\Jarvis /DOutName=Jarvis-Setup   (the Lite installer: /DOutName=Jarvis-Setup-Lite)
;
; One script for both installers. The first screen asks for the language of the installation (Russian or English);
; every screen of the installer then uses it: the standard texts come from the language files of Inno Setup,
; the texts of Jarvis (tasks, shortcuts, the last page) from [CustomMessages] below. The language of the
; installer does not change the language of Jarvis itself (that is a setting of the program).

[Setup]
AppName=Jarvis
AppVersion=0.1.0
AppPublisher=Jarvis
DefaultDirName={localappdata}\Programs\Jarvis
DefaultGroupName=Jarvis
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename={#OutName}
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
UninstallDisplayIcon={app}\jarvis-app.exe
SetupIconFile=..\resources\icons\icon.ico
; the language is always asked, even when Windows has one of them as its own language
ShowLanguageDialog=yes
LanguageDetectionMethod=uilanguage
; Jarvis running from a previous installation is closed by the installer (and started again from the last page)
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "russian"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Messages]
; the language dialog is shown before a language is chosen: its title and label are in both languages
russian.SelectLanguageTitle=Выберите язык установки / Select Setup Language
english.SelectLanguageTitle=Выберите язык установки / Select Setup Language
russian.SelectLanguageLabel=Выберите язык установки и нажмите «ОК». / Select the language to use during the installation.
english.SelectLanguageLabel=Выберите язык установки и нажмите «ОК». / Select the language to use during the installation.

[CustomMessages]
russian.WelcomeAbout=Джарвис — голосовой помощник для Windows: команды компьютера голосом и текстом, чат с ИИ, пульт с телефона.
english.WelcomeAbout=Jarvis is a voice assistant for Windows: computer commands by voice and text, AI chat, a phone remote.
russian.TaskDesktop=Создать ярлык на рабочем столе
english.TaskDesktop=Create a desktop shortcut
russian.TaskAutostart=Запускать Джарвис вместе с Windows (в фоне, значок в трее)
english.TaskAutostart=Start Jarvis with Windows (in the background, tray icon)
russian.TaskGroupExtra=Дополнительно:
english.TaskGroupExtra=Additional:
russian.ShortcutText=Jarvis — только текст, без микрофона
english.ShortcutText=Jarvis - text only, no microphone
russian.LaunchJarvis=Запустить JARVIS
english.LaunchJarvis=Launch JARVIS
russian.LaunchJarvisText=Запустить JARVIS в текстовом режиме (без микрофона)
english.LaunchJarvisText=Launch JARVIS in text mode (no microphone)
#if OutName == "Jarvis-Setup-Lite"
russian.EditionNote=Это облегчённая версия: в ней нет модели распознавания речи и голоса Piper, поэтому Джарвис работает с командами текстом и говорит голосом Windows. Для голосового управления установите полную версию (Jarvis-Setup.exe).
english.EditionNote=This is the Lite edition: it has no speech recognition model and no Piper voice, so Jarvis takes commands as text and speaks with a Windows voice. For voice control install the full edition (Jarvis-Setup.exe).
#else
russian.EditionNote=Скажите «Джарвис», затем команду, или напишите её в окне. Окно можно свернуть в трей — Джарвис продолжит слушать.
english.EditionNote=Say "Jarvis", then a command, or type it in the window. The window can be minimized to the tray — Jarvis keeps listening.
#endif

[Tasks]
Name: "desktopicon"; Description: "{cm:TaskDesktop}"; GroupDescription: "{cm:TaskGroupExtra}"; Flags: unchecked
Name: "autostart"; Description: "{cm:TaskAutostart}"; GroupDescription: "{cm:TaskGroupExtra}"; Flags: unchecked

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Jarvis"; ValueData: """{app}\jarvis-app.exe"" --background"; Flags: uninsdeletevalue; Tasks: autostart

[Files]
Source: "..\{#SrcDir}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{group}\Jarvis"; Filename: "{app}\jarvis-app.exe"; WorkingDir: "{app}"
Name: "{group}\{cm:ShortcutText}"; Filename: "{app}\jarvis-app.exe"; Parameters: "--text"; WorkingDir: "{app}"
Name: "{autodesktop}\Jarvis"; Filename: "{app}\jarvis-app.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
; the last page: the normal Jarvis (window, voice) is the main choice and is checked; the text mode is a second,
; unchecked choice. Both start the same program; a second start only shows the window of the running one.
Filename: "{app}\jarvis-app.exe"; WorkingDir: "{app}"; Description: "{cm:LaunchJarvis}"; Flags: postinstall nowait skipifsilent
Filename: "{app}\jarvis-app.exe"; Parameters: "--text"; WorkingDir: "{app}"; Description: "{cm:LaunchJarvisText}"; Flags: postinstall nowait skipifsilent unchecked

[Code]
// the description of Jarvis on the welcome page and what this edition can do on the last page, in the chosen language
procedure InitializeWizard();
begin
  WizardForm.WelcomeLabel2.Caption := WizardForm.WelcomeLabel2.Caption + #13#10#13#10 + CustomMessage('WelcomeAbout');
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpFinished then
    WizardForm.FinishedLabel.Caption := WizardForm.FinishedLabel.Caption + #13#10#13#10 + CustomMessage('EditionNote');
end;
