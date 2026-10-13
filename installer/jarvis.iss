; Inno Setup script. Run from the repository root: ISCC.exe installer\jarvis.iss
; The installer is written to the dist\ folder.
; Parameters: /DSrcDir=build\Jarvis /DOutName=Jarvis-Setup   (the Lite installer: /DOutName=Jarvis-Setup-Lite)
;
; One script for both installers. The first screen asks for the language of the installation (Russian or English);
; every screen of the installer then uses it: the standard texts come from the language files of Inno Setup,
; the texts of Jarvis (tasks, shortcuts, the last page) from [CustomMessages] below. The language of the
; installer does not change the language of Jarvis itself (that is a setting of the program).

#ifndef BuildSha
#define BuildSha "local"
#endif
#ifndef BuildDate
#define BuildDate "-"
#endif

[Setup]
AppName=Jarvis
AppVersion=0.1.0
; the commit and the date are shown in the title of the installer and in "Apps" of Windows: an old build is easy to tell
AppVerName=Jarvis 0.1.0 (build {#BuildSha}, {#BuildDate})
VersionInfoVersion=0.1.0.0
VersionInfoDescription=Jarvis installer, build {#BuildSha}
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
; Jarvis running from a previous installation is closed by the installer (and started again from the last page);
; "force": if it does not close by itself (it may run hidden in the tray), it is closed anyway
CloseApplications=force
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
russian.UninstallDataAsk=Удалить также ваши данные JARVIS на этом компьютере: настройки, историю диалогов, сценарии и журналы?%n%nСекреты (токен Telegram-бота и ссылка пульта телефона) удаляются в любом случае. Выберите «Нет», если собираетесь установить JARVIS снова и сохранить настройки.
english.UninstallDataAsk=Also delete your JARVIS data on this computer: settings, conversation history, scenarios and logs?%n%nThe secrets (the Telegram bot token and the phone remote link) are deleted in any case. Choose "No" if you plan to install JARVIS again and keep your settings.
#if OutName == "Jarvis-Setup-Lite"
russian.EditionNote=Это облегчённая версия: в ней нет модели распознавания речи и голоса Джарвиса (Piper), поэтому команды вводятся текстом, а ответы показываются в окне без звука. Озвучку можно включить в Настройки → Голос («Установить голос Джарвиса» или голос Windows). Для голосового управления установите полную версию (Jarvis-Setup.exe).
english.EditionNote=This is the Lite edition: it has no speech recognition model and no Jarvis voice (Piper), so commands are typed and answers are shown in the window without sound. Speech can be turned on in Settings → Voice ("Install the Jarvis voice" or a Windows voice). For voice control install the full edition (Jarvis-Setup.exe).
#else
russian.EditionNote=Скажите «Джарвис», затем команду, или напишите её в окне. Окно можно свернуть в трей — Джарвис продолжит слушать.
english.EditionNote=Say "Jarvis", then a command, or type it in the window. The window can be minimized to the tray — Jarvis keeps listening.
#endif

[Tasks]
Name: "desktopicon"; Description: "{cm:TaskDesktop}"; GroupDescription: "{cm:TaskGroupExtra}"; Flags: unchecked
Name: "autostart"; Description: "{cm:TaskAutostart}"; GroupDescription: "{cm:TaskGroupExtra}"; Flags: unchecked

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Jarvis"; ValueData: """{app}\jarvis-app.exe"" --background"; Flags: uninsdeletevalue; Tasks: autostart
; autostart switched on later in the settings of Jarvis writes the same value: the uninstaller removes it too
; (otherwise Windows would try to start a deleted program at every login)
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: none; ValueName: "Jarvis"; Flags: uninsdeletevalue

[UninstallRun]
; Jarvis running hidden in the tray keeps its files open: it is ended before its files are removed
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM jarvis-app.exe"; Flags: runhidden waituntilterminated; RunOnceId: "StopJarvis"

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
  WizardForm.AdjustLabelHeight(WizardForm.WelcomeLabel2);
end;

// the two ways to start on the last page exclude each other: the normal Jarvis or the text mode, never both
// (a second start would only show the window of the first one)
procedure RunListClickCheck(Sender: TObject);
var
  i: Integer;
begin
  i := WizardForm.RunList.ItemIndex;
  if (i >= 0) and (WizardForm.RunList.Items.Count = 2) and WizardForm.RunList.Checked[i] then
    WizardForm.RunList.Checked[1 - i] := False;
end;

procedure CurPageChanged(CurPageID: Integer);
var
  grown: Integer;
begin
  if CurPageID = wpFinished then
  begin
    WizardForm.FinishedLabel.Caption := WizardForm.FinishedLabel.Caption + #13#10#13#10 + CustomMessage('EditionNote');
    // the longer text gets its height and the choices move below it (no overlap)
    grown := WizardForm.AdjustLabelHeight(WizardForm.FinishedLabel);
    WizardForm.RunList.Top := WizardForm.RunList.Top + grown;
    WizardForm.RunList.OnClickCheck := @RunListClickCheck;
  end;
end;

// The program folder is changeable only by SYSTEM, Administrators and its owner (the user who installed it). A folder made
// under a wide parent (C:\ gives "Authenticated Users" the right to change in it) would let any other user of the
// computer replace jarvis-app.exe, a library or a command file that the owner then runs. The inherited rights are cut and
// the list is written anew: SIDs, so it does not depend on the language of Windows. Other users of the computer may
// read and run (a folder outside the profile of the installing user), never change. Inside the profile nothing is added
// (its usual list is the user's own already). The folder of the settings (%APPDATA%) is closed by the program itself.
procedure HardenAppFolder;
var
  rc: Integer;
  grants, app, profile, who: String;
  icacls: String;
begin
  app := ExpandConstant('{app}');
  profile := GetEnv('USERPROFILE');
  icacls := ExpandConstant('{sys}\icacls.exe');
  // the user who installs keeps the right to change (by name, DOMAIN\user: it is also right for a domain account)
  who := GetEnv('USERDOMAIN') + '\' + GetEnv('USERNAME');
  grants := '"' + app + '" /grant:r *S-1-5-18:(OI)(CI)F *S-1-5-32-544:(OI)(CI)F "' + who + '":(OI)(CI)M';
  if Pos(Lowercase(profile) + '\', Lowercase(app) + '\') <> 1 then
    grants := grants + ' *S-1-5-32-545:(OI)(CI)RX';
  // first the grants, and only when they were written the inherited rights are cut (never the other way: a failed grant after
  // the cut would leave the installing user without rights)
  if (not Exec(icacls, grants, '', SW_HIDE, ewWaitUntilTerminated, rc)) or (rc <> 0) then
  begin
    Log('Access rights of the program folder were not restricted: grant failed (icacls code ' + IntToStr(rc) + ')');
    Exit;
  end;
  if (not Exec(icacls, '"' + app + '" /inheritance:r', '', SW_HIDE, ewWaitUntilTerminated, rc)) or (rc <> 0) then
    Log('Access rights of the program folder were not restricted: inheritance not cut (icacls code ' + IntToStr(rc) + ')')
  else
    Log('Access rights of the program folder were restricted');
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  Log('Setup step ' + IntToStr(Ord(CurStep)) + ' (ssInstall=1, ssPostInstall=2, ssDone=3)');
  if CurStep = ssPostInstall then
    HardenAppFolder;
end;

// The data of Jarvis lives outside the program folder (%APPDATA%\com.priler.jarvis, config.rs BUNDLE_IDENTIFIER).
// After the uninstall the secrets are always removed (the token of the Telegram bot, the secret of the phone remote,
// the secret of the window and the certificate); the rest — settings, conversations, scenarios, logs — only when the user says so (a silent
// uninstall keeps them: the user did not answer). The tasks file in Documents belongs to the user and stays.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  dir: String;
begin
  if CurUninstallStep <> usPostUninstall then
    Exit;
  dir := ExpandConstant('{userappdata}\com.priler.jarvis');
  if not DirExists(dir) then
    Exit;
  DeleteFile(dir + '\telegram.json');
  DeleteFile(dir + '\remote_token');
  DeleteFile(dir + '\ui_token');
  DeleteFile(dir + '\remote_key.der');
  DeleteFile(dir + '\remote_cert.der');
  DeleteFile(dir + '\remote_cert.ip');
  // the downloaded program of the access from any network (about 55 MB): it is not the user's data
  DelTree(dir + '\cloudflared', True, True, True);
  if (not UninstallSilent) and (SuppressibleMsgBox(CustomMessage('UninstallDataAsk'), mbConfirmation, MB_YESNO or MB_DEFBUTTON2, IDNO) = IDYES) then
    DelTree(dir, True, True, True);
end;
