; AirWand installer (Inno Setup 6.3 or newer).
; Build:  ISCC.exe /DAppVersion=2.6.0 installer\AirWand.iss   (after `cargo build --release`)
; Signing: pass /Ssigntool="signtool.exe sign ... $f" and it signs the installer and the uninstaller.
; The release workflow does all of this (.github/workflows/release.yml).

#ifndef AppVersion
  #define AppVersion "2.6.0"
#endif
#define AppExe "airwand.exe"

[Setup]
; Never change AppId: it is how Windows recognises an upgrade of the same app.
AppId={{8C0B6C2E-4A53-4E7B-9D0A-3B5F7C1A2E64}
AppName=AirWand
AppVersion={#AppVersion}
AppVerName=AirWand {#AppVersion}
AppPublisher=AirWand contributors
VersionInfoVersion={#AppVersion}
VersionInfoProductName=AirWand
VersionInfoDescription=AirWand setup
DefaultDirName={autopf}\AirWand
DefaultGroupName=AirWand
DisableProgramGroupPage=yes
; Installs for the current user without a UAC prompt. The wizard offers "all users" for people who want it.
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.19041
OutputDir=..\dist
OutputBaseFilename=AirWand-Setup-{#AppVersion}
SetupIconFile=..\AppIcon.ico
UninstallDisplayIcon={app}\{#AppExe}
LicenseFile=..\..\LICENSE
Compression=lzma2/max
SolidCompression=yes
WizardStyle=modern
; The app holds this mutex while it runs; setup asks to close it before replacing the file.
AppMutex=AirWand.SingleInstance
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "startup"; Description: "Start AirWand when I sign in to Windows (runs in the tray)"; Flags: unchecked
Name: "desktopicon"; Description: "Create a &desktop shortcut"; Flags: unchecked

[Files]
Source: "..\target\release\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\Web\Geist-OFL.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\AirWand"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\AirWand"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "AirWand"; ValueData: """{app}\{#AppExe}"" --background"; Tasks: startup; Flags: uninsdeletevalue

[Run]
; Only when installed for all users (administrator): let AirWand through the firewall on private networks,
; so the phone can reach it without a prompt. Per-user installs get Windows' own prompt on first start.
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall add rule name=""AirWand"" dir=in action=allow program=""{app}\{#AppExe}"" profile=private enable=yes"; Flags: runhidden; Check: IsAdminInstallMode; StatusMsg: "Allowing AirWand on private networks..."
Filename: "{app}\{#AppExe}"; Description: "Start AirWand"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{sys}\netsh.exe"; Parameters: "advfirewall firewall delete rule name=""AirWand"" program=""{app}\{#AppExe}"""; Flags: runhidden; RunOnceId: "AirWandFirewall"; Check: IsAdminInstallMode

[Code]
// The app can switch "Start with Windows" on by itself; remove that entry too when uninstalling.
procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
begin
  if CurUninstallStep = usPostUninstall then
    RegDeleteValue(HKEY_CURRENT_USER, 'Software\Microsoft\Windows\CurrentVersion\Run', 'AirWand');
end;
