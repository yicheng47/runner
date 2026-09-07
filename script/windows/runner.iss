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
#ifndef UpdatesUrl
  #define UpdatesUrl "https://github.com/yicheng47/runner/releases/tag/nightly-win"
#endif

[Setup]
AppId={#AppId}
AppName={#AppName}
AppVersion={#AppVersion}
AppPublisher=wyc studios
AppPublisherURL=https://github.com/yicheng47/runner
AppUpdatesURL={#UpdatesUrl}
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
Filename: "{app}\Runner.exe"; WorkingDir: "{%USERPROFILE}"; Flags: nowait; Check: RelaunchRequested

[Code]
var
  InstallCompleted: Boolean;

function OpenProcess(DesiredAccess: LongWord; InheritHandle: Boolean; ProcessId: LongWord): THandle;
  external 'OpenProcess@kernel32.dll stdcall';
function WaitForSingleObject(Handle: THandle; Milliseconds: LongWord): LongWord;
  external 'WaitForSingleObject@kernel32.dll stdcall';
function CloseHandle(Handle: THandle): Boolean;
  external 'CloseHandle@kernel32.dll stdcall';

function RelaunchRequested: Boolean;
begin
  Result := ExpandConstant('{param:RELAUNCH|0}') = '1';
end;

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
var
  ProcessId: Integer;
  ProcessHandle: THandle;
  WaitResult: LongWord;
begin
  Result := '';
  ProcessId := StrToIntDef(ExpandConstant('{param:WAITPID|0}'), 0);
  if ProcessId <> 0 then begin
    ProcessHandle := OpenProcess($00100000, False, ProcessId);
    if ProcessHandle <> 0 then begin
      Log('Waiting for Runner process ' + IntToStr(ProcessId));
      WaitResult := WaitForSingleObject(ProcessHandle, 30000);
      CloseHandle(ProcessHandle);
      Log('Runner process wait finished: ' + IntToStr(WaitResult));
    end;
  end;
  if ApplicationFilesInUse then
    Result := 'Runner files are in use. Close Runner normally and finish any running CLI commands, then try again. You can cancel to keep working.';
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssDone then
    InstallCompleted := True;
end;

procedure DeinitializeSetup;
var
  ResultCode: Integer;
begin
  if RelaunchRequested and not InstallCompleted and FileExists(ExpandConstant('{app}\Runner.exe')) then
    Exec(ExpandConstant('{app}\Runner.exe'), '', ExpandConstant('{%USERPROFILE}'), SW_SHOWNORMAL, ewNoWait, ResultCode);
end;

function InitializeUninstall: Boolean;
begin
  Result := not ApplicationFilesInUse;
  if not Result then
    SuppressibleMsgBox('Runner files are in use. Close Runner normally and finish any running CLI commands, then run uninstall again.', mbError, MB_OK, IDOK);
end;
