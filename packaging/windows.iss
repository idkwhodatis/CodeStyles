; Invoked only after package_app.py verifies and stages the complete offline app.
; Inno Setup 6.3+ accepts x64compatible and arm64 architecture identifiers.
#ifndef SourceDir
  #error SourceDir must point to the verified application staging directory
#endif
#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef AppArch
  #error AppArch is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif
#ifndef OutputBase
  #error OutputBase is required
#endif

[Setup]
AppId={{D4DB6CB7-38B0-4E1F-BD22-01679836B2EC}
AppName=CodeStyles
AppVersion={#AppVersion}
AppPublisher=CodeStyles contributors
AppPublisherURL=https://github.com/idkwhodatis/CodeStyles
AppSupportURL=https://github.com/idkwhodatis/CodeStyles/issues
DefaultDirName={localappdata}\Programs\CodeStyles
DefaultGroupName=CodeStyles
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed={#AppArch}
ArchitecturesInstallIn64BitMode={#AppArch}
; Inno 7+ can emit an x64 helper, which would exclude Windows 10 ARM64.
; Earlier supported Inno versions always use the x86 helper.
#if VER >= EncodeVer(7, 0, 0)
SetupArchitecture=x86
#endif
MinVersion=10.0.19045
LicenseFile={#SourceDir}\LICENSE
UninstallDisplayIcon={app}\codestyles.exe
OutputDir={#OutputDir}
OutputBaseFilename={#OutputBase}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; GroupDescription: "Additional shortcuts:"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\CodeStyles"; Filename: "{app}\codestyles.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\CodeStyles"; Filename: "{app}\codestyles.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\codestyles.exe"; Description: "Launch CodeStyles"; Flags: nowait postinstall skipifsilent unchecked
