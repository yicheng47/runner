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
  #define UpdatesUrl "https://github.com/yicheng47/runner/releases/tag/nightly"
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
; Launch through the shell, never as a child of Setup: Setup runs under Redirection
; Guard and children inherit it, which makes user-created junctions (Codex's bin,
; scoop, pnpm) untraversable in the launched app.
Filename: "{win}\explorer.exe"; Parameters: """{app}\Runner.exe"""; Description: "Launch {#AppName}"; Flags: nowait postinstall unchecked skipifsilent
Filename: "{win}\explorer.exe"; Parameters: """{app}\Runner.exe"""; Flags: nowait; Check: RelaunchRequested

[Code]
var
  InstallCompleted: Boolean;
  RenamedFrom: TArrayOfString;
  RenamedTo: TArrayOfString;

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

procedure DeleteOldApplicationFiles;
var
  OldFile: TFindRec;
  Path: String;
begin
  if FindFirst(ExpandConstant('{app}\*.old'), OldFile) then begin
    try
      repeat
        if OldFile.Attributes and FILE_ATTRIBUTE_DIRECTORY = 0 then begin
          Path := AddBackslash(ExpandConstant('{app}')) + OldFile.Name;
          if DeleteFile(Path) then
            Log('Deleted old application file: ' + Path)
          else
            Log('Keeping old application file: ' + Path);
        end;
      until not FindNext(OldFile);
    finally
      FindClose(OldFile);
    end;
  end;
end;

function RenameApplicationFile(const Path: String): Boolean;
var
  OldPath: String;
  Suffix, Index: Integer;
begin
  OldPath := Path + '.old';
  Suffix := 0;
  while FileExists(OldPath) do begin
    if DeleteFile(OldPath) then
      Break;
    Suffix := Suffix + 1;
    OldPath := Path + '.' + IntToStr(Suffix) + '.old';
  end;
  Result := RenameFile(Path, OldPath);
  if Result then begin
    Index := GetArrayLength(RenamedFrom);
    SetArrayLength(RenamedFrom, Index + 1);
    SetArrayLength(RenamedTo, Index + 1);
    RenamedFrom[Index] := Path;
    RenamedTo[Index] := OldPath;
    Log('Renamed application file: ' + Path + ' -> ' + OldPath)
  end else
    Log('Cannot prepare application file: ' + Path);
end;

procedure RestoreRenamedApplicationFiles;
var
  I: Integer;
begin
  for I := GetArrayLength(RenamedFrom) - 1 downto 0 do begin
    if not FileExists(RenamedFrom[I]) and FileExists(RenamedTo[I]) then begin
      if RenameFile(RenamedTo[I], RenamedFrom[I]) then
        Log('Restored application file: ' + RenamedFrom[I])
      else
        Log('Cannot restore application file: ' + RenamedFrom[I]);
    end;
  end;
end;

function PrepareApplicationFiles: String;
var
  Names: TArrayOfString;
  I: Integer;
  FileStream: TFileStream;
  Path: String;
begin
  Result := '';
  Names := ['Runner.exe', 'runner-agent-cli.exe', 'runner-mcp.exe'];
  for I := 0 to GetArrayLength(Names) - 1 do begin
    Path := AddBackslash(ExpandConstant('{app}')) + Names[I];
    if FileExists(Path) then begin
      try
        FileStream := TFileStream.Create(Path, fmOpenReadWrite or fmShareDenyNone);
        FileStream.Free;
      except
        if not RenameApplicationFile(Path) then begin
          Result := 'Could not move ' + Path + ' aside. Check file permissions and any file locks, then try again.';
          RestoreRenamedApplicationFiles;
          Exit;
        end;
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
  DeleteOldApplicationFiles;
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
  Result := PrepareApplicationFiles;
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
  if not InstallCompleted then
    RestoreRenamedApplicationFiles;
  if RelaunchRequested and not InstallCompleted and FileExists(ExpandConstant('{app}\Runner.exe')) then
    Exec(ExpandConstant('{win}\explorer.exe'), '"' + ExpandConstant('{app}\Runner.exe') + '"', '', SW_SHOWNORMAL, ewNoWait, ResultCode);
end;

function InitializeUninstall: Boolean;
begin
  DeleteOldApplicationFiles;
  Result := True;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Error: String;
begin
  if CurUninstallStep = usUninstall then begin
    Error := PrepareApplicationFiles;
    if Error <> '' then begin
      SuppressibleMsgBox(Error, mbError, MB_OK, IDOK);
      Abort;
    end;
  end;
end;
