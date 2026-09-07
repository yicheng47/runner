param([string]$SourceDir)

$ErrorActionPreference = 'Stop'
if ($SourceDir) { $SourceDir = (Resolve-Path -LiteralPath $SourceDir).Path }
$compiler = & (Join-Path $PSScriptRoot 'inno-setup.ps1')
$id = [Guid]::NewGuid().ToString('N')
$appId = "runner-installer-test.$id"
$appName = "Runner Installer Test $id"
$testRoot = Join-Path ([IO.Path]::GetTempPath()) "runner installer % # $id"
$installDir = Join-Path $testRoot 'installed'
$registryKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\${appId}_is1"
$shortcutPath = Join-Path ([Environment]::GetFolderPath('Programs')) "$appName.lnk"
$binaries = @('Runner.exe', 'runner-agent-cli.exe', 'runner-mcp.exe')
$csc = Join-Path $env:WINDIR 'Microsoft.NET/Framework64/v4.0.30319/csc.exe'
New-Item -ItemType Directory -Path $testRoot | Out-Null
Write-Host "Installer smoke artifacts: $testRoot"

function Invoke-Setup([string]$Path, [string]$Label, [string[]]$Extra = @(), [switch]$ExpectBlocked) {
    $waitForTree = -not ($Extra -contains '/RELAUNCH=1')
    $process = Start-Process -FilePath $Path -ArgumentList (@(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$testRoot/$Label.log`""
    ) + $Extra) -WindowStyle Hidden -Wait:$waitForTree -PassThru
    $process.WaitForExit()
    if ($ExpectBlocked) {
        if ($process.ExitCode -eq 0) { throw "$Label should refuse files that are in use" }
        if (-not (Select-String -LiteralPath "$testRoot/$Label.log" -SimpleMatch 'Cannot replace application file:' -Quiet)) {
            throw "$Label failed without detecting the locked executable"
        }
    } elseif ($process.ExitCode -ne 0) {
        throw "$Label failed with exit code $($process.ExitCode); see $testRoot/$Label.log"
    }
}

function Assert-Installed([string]$Stage, [string]$Version, [string]$UpdatesUrl = 'https://github.com/yicheng47/runner/releases/tag/nightly-win') {
    foreach ($binary in $binaries) {
        $expected = (Get-FileHash -LiteralPath (Join-Path $Stage $binary)).Hash
        $actual = (Get-FileHash -LiteralPath (Join-Path $installDir $binary)).Hash
        if ($actual -ne $expected) { throw "$binary was not replaced by $Version" }
    }
    $entry = Get-ItemProperty -LiteralPath $registryKey
    if ($entry.DisplayVersion -ne $Version -or $entry.InstallLocation.TrimEnd('\') -ne $installDir) {
        throw 'Uninstall entry has the wrong version or installation directory'
    }
    if ($entry.URLUpdateInfo -ne $UpdatesUrl) { throw 'Uninstall entry has the wrong update channel' }
    $shortcut = (New-Object -ComObject WScript.Shell).CreateShortcut($shortcutPath)
    if ($shortcut.TargetPath -ne (Join-Path $installDir 'Runner.exe')) { throw 'Start Menu target differs' }
    if ($shortcut.WorkingDirectory -ne [Environment]::GetFolderPath('UserProfile')) { throw 'Start Menu cwd is not home' }
}

foreach ($revision in 1, 2) {
    $stage = Join-Path $testRoot "payload-$revision"
    New-Item -ItemType Directory -Path $stage | Out-Null
    if ($SourceDir) {
        foreach ($binary in $binaries) {
            $destination = Join-Path $stage $binary
            Copy-Item -LiteralPath (Join-Path $SourceDir $binary) -Destination $destination
            if ($revision -eq 1) {
                $stream = [IO.File]::Open($destination, [IO.FileMode]::Append)
                try { $stream.WriteByte(0) } finally { $stream.Dispose() }
            }
        }
    } else {
        $source = Join-Path $stage 'fixture.cs'
        @"
using System.Reflection;
[assembly: AssemblyVersion("0.7.5.0")]
[assembly: AssemblyFileVersion("0.7.5.0")]
[assembly: AssemblyDescription("nightly $revision")]
class Fixture { static void Main() { System.Threading.Thread.Sleep(60000); } }
"@ | Set-Content -LiteralPath $source -Encoding UTF8
        & $csc /nologo /target:winexe /platform:x64 "/out:$stage/Runner.exe" $source
        if ($LASTEXITCODE -ne 0) { throw 'Could not compile installer test executable' }
        foreach ($binary in $binaries | Select-Object -Skip 1) {
            Copy-Item -LiteralPath "$stage/Runner.exe" -Destination (Join-Path $stage $binary)
        }
    }
    $updatesUrl = if ($revision -eq 2) {
        'https://github.com/yicheng47/runner/releases/latest'
    } else {
        'https://github.com/yicheng47/runner/releases/tag/nightly-win'
    }
    & $compiler /Q "/DAppId=$appId" "/DAppName=$appName" "/DAppVersion=0.7.5.20260101.000$revision" /DBaseVersion=0.7.5 "/DUpdatesUrl=$updatesUrl" "/DSourceDir=$stage" "/DOutputDir=$testRoot" (Join-Path $PSScriptRoot 'runner.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Installer test compilation failed' }
}

$first = Join-Path $testRoot 'Runner-Setup-0.7.5.20260101.0001-x64.exe'
$second = Join-Path $testRoot 'Runner-Setup-0.7.5.20260101.0002-x64.exe'
$uninstaller = Join-Path $installDir 'unins000.exe'
Invoke-Setup $first 'fresh-install' @("/DIR=`"$installDir`"")
Assert-Installed "$testRoot/payload-1" '0.7.5.20260101.0001'

$retainedFile = Join-Path $installDir 'retained-user-file.txt'
Set-Content -LiteralPath $retainedFile -Value 'keep across upgrade and uninstall' -Encoding UTF8
foreach ($binary in $binaries) {
    $lock = [IO.File]::Open((Join-Path $installDir $binary), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
    try {
        Invoke-Setup $second "blocked-upgrade-$binary" -ExpectBlocked
        Invoke-Setup $uninstaller "blocked-uninstall-$binary" -ExpectBlocked
        Assert-Installed "$testRoot/payload-1" '0.7.5.20260101.0001'
    } finally {
        $lock.Dispose()
    }
    (Get-Item -LiteralPath (Join-Path $installDir $binary)).LastWriteTimeUtc = [DateTime]::UtcNow.AddDays(1)
}
Invoke-Setup $second 'upgrade'
Assert-Installed "$testRoot/payload-2" '0.7.5.20260101.0002' 'https://github.com/yicheng47/runner/releases/latest'
Write-Host 'PASS: fresh install, shortcut, per-user registration, blocked upgrades/uninstalls, same-version binary replacement, nightly-to-production channel switch'

if (-not $SourceDir) {
    $helperSource = Join-Path $testRoot 'lock-helper.cs'
    $helperExe = Join-Path $testRoot 'lock-helper.exe'
    @'
using System.IO;
using System.Threading;
class LockHelper {
    static void Main(string[] args) {
        using (var file = File.Open(args[0], FileMode.Open, FileAccess.Read, FileShare.Read)) {
            File.WriteAllText(args[1], "ready");
            Thread.Sleep(int.Parse(args[2]));
        }
    }
}
'@ | Set-Content -LiteralPath $helperSource -Encoding UTF8
    & $csc /nologo /target:winexe /platform:x64 "/out:$helperExe" $helperSource
    if ($LASTEXITCODE -ne 0) { throw 'Could not compile installer lock helper' }

    function Start-LockHelper([string]$Label, [int]$Milliseconds) {
        $ready = Join-Path $testRoot "$Label.ready"
        $helper = Start-Process -FilePath $helperExe -ArgumentList @(
            "`"$installDir/Runner.exe`"", "`"$ready`"", "$Milliseconds"
        ) -WindowStyle Hidden -PassThru
        for ($attempt = 0; $attempt -lt 100; $attempt++) {
            if (Test-Path -LiteralPath $ready) { return $helper }
            Start-Sleep -Milliseconds 50
        }
        if (-not $helper.HasExited) { $helper.Kill(); $helper.WaitForExit() }
        throw 'Lock helper did not open the fixture'
    }

    function Stop-RelaunchedFixture {
        $fixturePath = Join-Path $installDir 'Runner.exe'
        for ($attempt = 0; $attempt -lt 100; $attempt++) {
            $processes = @(Get-Process -Name Runner -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $fixturePath })
            if ($processes.Count -gt 0) {
                foreach ($process in $processes) { $process.Kill(); $process.WaitForExit() }
                if ($processes.Count -ne 1) { throw 'Installer relaunched the fixture more than once' }
                return
            }
            Start-Sleep -Milliseconds 50
        }
        throw 'Installer did not relaunch the installed Runner.exe fixture'
    }

    $helper = Start-LockHelper 'waitpid' 3000
    try {
        Invoke-Setup $second 'waitpid' @("/WAITPID=$($helper.Id)")
        if (-not (Select-String -LiteralPath "$testRoot/waitpid.log" -SimpleMatch "Waiting for Runner process $($helper.Id)" -Quiet) -or
            -not (Select-String -LiteralPath "$testRoot/waitpid.log" -SimpleMatch 'Runner process wait finished: 0' -Quiet)) {
            throw 'Installer did not wait for the live helper to exit'
        }
    } finally {
        if (-not $helper.HasExited) { $helper.Kill(); $helper.WaitForExit() }
    }
    Assert-Installed "$testRoot/payload-2" '0.7.5.20260101.0002' 'https://github.com/yicheng47/runner/releases/latest'

    try {
        Invoke-Setup $second 'relaunch-success' @('/RELAUNCH=1')
    } finally {
        Stop-RelaunchedFixture
    }

    $helper = Start-LockHelper 'relaunch-refusal' 60000
    try {
        Invoke-Setup $second 'relaunch-refusal' @('/RELAUNCH=1') -ExpectBlocked
    } finally {
        try { Stop-RelaunchedFixture } finally {
            if (-not $helper.HasExited) { $helper.Kill(); $helper.WaitForExit() }
        }
    }
    Assert-Installed "$testRoot/payload-2" '0.7.5.20260101.0002' 'https://github.com/yicheng47/runner/releases/latest'
    Write-Host 'PASS: WAITPID waits for exit, silent install relaunches once, refused install relaunches the old fixture'
}

Invoke-Setup $uninstaller 'uninstall'
if ((Test-Path -LiteralPath $registryKey) -or (Test-Path -LiteralPath $shortcutPath)) { throw 'Uninstall registration or shortcut remains' }
foreach ($binary in $binaries) {
    if (Test-Path -LiteralPath (Join-Path $installDir $binary)) { throw "Uninstall left $binary" }
}
if ((Get-Content -LiteralPath $retainedFile -Raw).Trim() -ne 'keep across upgrade and uninstall') { throw 'Uninstall removed unowned data' }

Invoke-Setup $second 'reinstall' @("/DIR=`"$installDir`"")
Assert-Installed "$testRoot/payload-2" '0.7.5.20260101.0002' 'https://github.com/yicheng47/runner/releases/latest'
if (-not (Test-Path -LiteralPath $retainedFile)) { throw 'Reinstall removed retained data' }
Invoke-Setup $uninstaller 'final-uninstall'
Write-Host 'PASS: uninstall removes binaries/shortcut/entry, retains unowned data, and reinstall succeeds'
