; Inno Setup script. Run from the repository root: ISCC.exe installer\jarvis.iss
; The installer is written to the dist\ folder.

[Setup]
AppName=Jarvis
AppVersion=0.1.0
AppPublisher=Jarvis
DefaultDirName={localappdata}\Programs\Jarvis
DefaultGroupName=Jarvis
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=Jarvis-Setup
Compression=lzma2
SolidCompression=yes
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
UninstallDisplayIcon={app}\jarvis-app.exe

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "..\build\Jarvis\*"; DestDir: "{app}"; Flags: recursesubdirs createallsubdirs ignoreversion

[Icons]
Name: "{group}\Jarvis"; Filename: "{app}\jarvis-app.exe"; WorkingDir: "{app}"
Name: "{group}\Jarvis (text mode)"; Filename: "{app}\jarvis-app.exe"; Parameters: "--text"; WorkingDir: "{app}"
Name: "{autodesktop}\Jarvis"; Filename: "{app}\jarvis-app.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\jarvis-app.exe"; Parameters: "--text"; WorkingDir: "{app}"; Description: "Start Jarvis (text mode)"; Flags: postinstall nowait skipifsilent unchecked
