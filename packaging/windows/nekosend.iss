#ifndef BundleDir
  #error BundleDir is required
#endif
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif
#ifndef ShellIdentityRepair
  #define ShellIdentityRepair 0
#endif

[Setup]
AppId={{760AA855-DB24-4DF3-8D75-45573FC01949}
AppName=NekoSend
AppVersion={#AppVersion}
AppPublisher=iris-Neko
AppPublisherURL=https://github.com/iris-Neko/NekoSend
AppSupportURL=https://github.com/iris-Neko/NekoSend/issues
DefaultDirName={localappdata}\Programs\LAN Chat
DefaultGroupName=NekoSend
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir={#OutputDir}
OutputBaseFilename=NekoSend-windows-x64-setup
SetupIconFile={#BundleDir}\..\..\..\..\..\windows\runner\resources\app_icon.ico
UninstallDisplayIcon={app}\lan_chat.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
DisableProgramGroupPage=yes
AppMutex=Local\LANChat.SingleInstance.V1
CloseApplications=no
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Files]
Source: "{#BundleDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{userprograms}\NekoSend"; Filename: "{app}\lan_chat.exe"; WorkingDir: "{app}"; IconFilename: "{app}\lan_chat.exe"; AppUserModelID: "dev.lanchat.LANChat"
Name: "{userdesktop}\NekoSend"; Filename: "{app}\lan_chat.exe"; WorkingDir: "{app}"; IconFilename: "{app}\lan_chat.exe"; AppUserModelID: "dev.lanchat.LANChat"; Tasks: desktopicon

[Run]
#if ShellIdentityRepair
Filename: "{app}\lan_chat.exe"; Parameters: "--repair-shell-identity"; Flags: runhidden waituntilterminated
#endif
Filename: "{app}\lan_chat.exe"; Description: "Launch NekoSend"; Flags: nowait postinstall skipifsilent

; No user-data directory is listed for deletion: chat history survives uninstall.
[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "LAN Chat"; Flags: uninsdeletevalue
