#ifndef AppVersion
  #error AppVersion is required
#endif
#ifndef BaseVersion
  #error BaseVersion is required
#endif
#ifndef SourceDir
  #error SourceDir is required
#endif
#ifndef OutputDir
  #error OutputDir is required
#endif
#ifndef AppId
  #define AppId "com.wycstudios.runner"
#endif
#ifndef AppName
  #define AppName "Runner"
#endif

[Setup]
AppId={#AppId}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=wyc studios
AppPublisherURL=https://github.com/yicheng47/runner
AppUpdatesURL=https://github.com/yicheng47/runner/releases/tag/nightly-win
DefaultDirName={localappdata}\Programs\{#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
OutputDir={#OutputDir}
OutputBaseFilename=Runner-Setup-{#AppVersion}-x64
SetupIconFile=..\..\assets\icon.ico
UninstallDisplayIcon={app}\Runner.exe
VersionInfoVersion={#BaseVersion}
VersionInfoProductVersion={#BaseVersion}
VersionInfoProductTextVersion={#AppVersion}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
CloseApplications=no
RestartApplications=no
SetupMutex={#AppId}.setup

[Files]
; Nightlies share file versions; every upgrade must replace all three binaries.
Source: "{#SourceDir}\Runner.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\runner-agent-cli.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\runner-mcp.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\crates\runner-app\LICENSE.xterm"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\fonts\JetBrainsMono-NF-LICENSE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\fonts\JetBrainsMono-NF-NOTICE.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\..\assets\fonts\OFL.txt"; DestDir: "{app}"; Flags: ignoreversion

[Icons]
Name: "{userprograms}\{#AppName}"; Filename: "{app}\Runner.exe"; WorkingDir: "{%USERPROFILE}"; AppUserModelID: "{#AppId}"

[Run]
Filename: "{app}\Runner.exe"; WorkingDir: "{%USERPROFILE}"; Description: "Launch {#AppName}"; Flags: nowait postinstall unchecked skipifsilent

[Code]
function ApplicationFilesInUse: Boolean;
var
  Names: TArrayOfString;
  I: Integer;
  FileStream: TFileStream;
  Path: String;
begin
  Result := False;
  Names := ['Runner.exe', 'runner-agent-cli.exe', 'runner-mcp.exe'];
  for I := 0 to GetArrayLength(Names) - 1 do begin
    Path := AddBackslash(ExpandConstant('{app}')) + Names[I];
    if FileExists(Path) then begin
      try
        FileStream := TFileStream.Create(Path, fmOpenReadWrite or fmShareDenyNone);
        FileStream.Free;
      except
        Log('Cannot replace application file: ' + Path);
        Result := True;
        Exit;
      end;
    end;
  end;
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := '';
  if ApplicationFilesInUse then
    Result := 'Runner files are in use. Close Runner normally and finish any running CLI commands, then try again. You can cancel to keep working.';
end;

function InitializeUninstall: Boolean;
begin
  Result := not ApplicationFilesInUse;
  if not Result then
    SuppressibleMsgBox('Runner files are in use. Close Runner normally and finish any running CLI commands, then run uninstall again.', mbError, MB_OK, IDOK);
end;
