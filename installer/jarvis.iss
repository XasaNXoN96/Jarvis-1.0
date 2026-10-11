; Inno Setup script. Run from the repository root: ISCC.exe installer\jarvis.iss
; The installer is written to the dist\ folder.
; Parameters: /DSrcDir=build\Jarvis /DOutName=Jarvis-Setup

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

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked
Name: "autostart"; Description: "Start Jarvis with Windows (in the background, tray icon)"; Flags: unchecked

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Jarvis"; ValueData: """{app}\jarvis-app.exe"" --background"; Flags: uninsdeletevalue; Tasks: autostart

[Files]
Source: "..\{#SrcDir}\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{group}\Jarvis"; Filename: "{app}\jarvis-app.exe"; WorkingDir: "{app}"
Name: "{group}\Jarvis (text mode)"; Filename: "{app}\jarvis-app.exe"; Parameters: "--text"; WorkingDir: "{app}"
Name: "{autodesktop}\Jarvis"; Filename: "{app}\jarvis-app.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\jarvis-app.exe"; Parameters: "--text"; WorkingDir: "{app}"; Description: "Start Jarvis (text mode)"; Flags: postinstall nowait skipifsilent unchecked
