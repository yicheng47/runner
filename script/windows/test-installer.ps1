param([string]$SourceDir, [string]$SigningThumbprint = $env:CERTUM_CERTIFICATE_SHA1)

$ErrorActionPreference = 'Stop'
if ($SourceDir) { $SourceDir = (Resolve-Path -LiteralPath $SourceDir).Path }
$compiler = & (Join-Path $PSScriptRoot 'inno-setup.ps1')
$signed = [bool]($SourceDir -and $SigningThumbprint)
$signing = @()
if ($signed) {
    $signtool = & (Join-Path $PSScriptRoot 'signtool.ps1')
    $signing = @('/DSign', ('/Sauthenticode=$q{0}$q sign /sha1 {1} /fd sha256 /tr http://time.certum.pl /td sha256 $f' -f $signtool, $SigningThumbprint))
}
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

function Invoke-Setup([string]$Path, [string]$Label, [string[]]$Extra = @(), [switch]$ExpectBlocked, [switch]$ExpectFailure) {
    $waitForTree = -not ($Extra -contains '/RELAUNCH=1')
    $process = Start-Process -FilePath $Path -ArgumentList (@(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', "/LOG=`"$testRoot/$Label.log`""
    ) + $Extra) -WindowStyle Hidden -Wait:$waitForTree -PassThru
    $process.WaitForExit()
    if ($ExpectBlocked -or $ExpectFailure) {
        if ($process.ExitCode -eq 0) { throw "$Label should fail" }
        if ($ExpectBlocked -and -not (Select-String -LiteralPath "$testRoot/$Label.log" -SimpleMatch 'Cannot prepare application file:' -Quiet)) {
            throw "$Label failed without detecting the locked executable"
        }
    } elseif ($process.ExitCode -ne 0) {
        throw "$Label failed with exit code $($process.ExitCode); see $testRoot/$Label.log"
    }
}

function Assert-Signed([string]$Path) {
    $signature = Get-AuthenticodeSignature -LiteralPath $Path
    if ($signature.Status -ne 'Valid') { throw "$Path signature is $($signature.Status)" }
    if ($signature.SignerCertificate.Thumbprint -ne $SigningThumbprint) { throw "$Path is signed by $($signature.SignerCertificate.Thumbprint), not $SigningThumbprint" }
    if (-not $signature.TimeStamperCertificate) { throw "$Path signature is not timestamped" }
}

function Assert-Installed([string]$Stage, [string]$Version, [string]$UpdatesUrl = 'https://github.com/yicheng47/runner/releases/tag/nightly') {
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
class Fixture {
    static void Main(string[] args) {
        if (args.Length > 0) System.IO.File.WriteAllText(args[0], "ready");
        System.Threading.Thread.Sleep(args.Length > 1 ? int.Parse(args[1]) : 60000);
    }
}
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
        'https://github.com/yicheng47/runner/releases/tag/nightly'
    }
    & $compiler /Q @signing "/DAppId=$appId" "/DAppName=$appName" "/DAppVersion=0.7.5.20260101.000$revision" /DBaseVersion=0.7.5 "/DUpdatesUrl=$updatesUrl" "/DSourceDir=$stage" "/DOutputDir=$testRoot" (Join-Path $PSScriptRoot 'runner.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Installer test compilation failed' }
}

$first = Join-Path $testRoot 'Runner-Setup-0.7.5.20260101.0001-x64.exe'
$second = Join-Path $testRoot 'Runner-Setup-0.7.5.20260101.0002-x64.exe'
$uninstaller = Join-Path $installDir 'unins000.exe'
Invoke-Setup $first 'fresh-install' @("/DIR=`"$installDir`"")
Assert-Installed "$testRoot/payload-1" '0.7.5.20260101.0001'

$retainedFile = Join-Path $installDir 'retained-user-file.txt'
Set-Content -LiteralPath $retainedFile -Value 'keep across upgrade and uninstall' -Encoding UTF8
if ($SourceDir) {
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
}
Invoke-Setup $second 'upgrade'
Assert-Installed "$testRoot/payload-2" '0.7.5.20260101.0002' 'https://github.com/yicheng47/runner/releases/latest'
Write-Host 'PASS: fresh install, shortcut, per-user registration, same-version binary replacement, nightly-to-production channel switch'
if ($signed) {
    foreach ($path in @($second) + @($binaries | ForEach-Object { Join-Path $installDir $_ }) + @($uninstaller)) { Assert-Signed $path }
    Write-Host 'PASS: installer, application binaries, and uninstaller carry valid timestamped signatures'
}

if (-not $SourceDir) {
    Add-Type @'
using System;
using System.Runtime.InteropServices;
public static class InstallerTestWindow {
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr FindWindow(string className, string title);
    [DllImport("user32.dll", CharSet = CharSet.Unicode)]
    public static extern IntPtr SendMessage(IntPtr window, uint message, IntPtr wParam, IntPtr lParam);
}
'@

    function Start-LockHelper([string]$Label, [int]$Milliseconds, [string]$Binary = 'Runner.exe') {
        $ready = Join-Path $testRoot "$Label.ready"
        $helper = Start-Process -FilePath (Join-Path $installDir $Binary) -ArgumentList @(
            "`"$ready`"", "$Milliseconds"
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
                try {
                    if ($processes.Count -ne 1) { throw 'Installer relaunched the fixture more than once' }
                } finally {
                    foreach ($process in $processes) { $process.Kill(); $process.WaitForExit() }
                    Start-Sleep -Milliseconds 250
                    $remaining = @(Get-Process -Name Runner -ErrorAction SilentlyContinue | Where-Object { $_.Path -eq $fixturePath })
                    if ($remaining.Count -gt 0) {
                        foreach ($process in $remaining) { $process.Kill(); $process.WaitForExit() }
                        throw 'Installer left an additional relaunched fixture running'
                    }
                }
                return
            }
            Start-Sleep -Milliseconds 50
        }
        throw 'Installer did not relaunch the installed Runner.exe fixture'
    }

    foreach ($binary in $binaries) {
        Invoke-Setup $first "baseline-$binary"
        $helper = Start-LockHelper "locked-$binary" 180000 $binary
        $waiter = $null
        try {
            $extra = @()
            if ($binary -eq 'Runner.exe') {
                $waiter = Start-LockHelper 'waitpid' 3000 'runner-agent-cli.exe'
                $extra = @("/WAITPID=$($waiter.Id)")
            }
            Invoke-Setup $second "locked-upgrade-$binary" $extra
            Assert-Installed "$testRoot/payload-2" '0.7.5.20260101.0002' 'https://github.com/yicheng47/runner/releases/latest'
            $oldPath = Join-Path $installDir "$binary.old"
            if ($helper.HasExited -or -not (Test-Path -LiteralPath $oldPath)) { throw "$binary was not kept running from its renamed file" }
            if ((Get-FileHash -LiteralPath $oldPath).Hash -ne (Get-FileHash -LiteralPath "$testRoot/payload-1/$binary").Hash) {
                throw "$binary.old does not contain the old binary"
            }
            if ($waiter) {
                $log = Get-Content -LiteralPath "$testRoot/locked-upgrade-$binary.log" -Raw
                $waitFinished = $log.IndexOf('Runner process wait finished: 0')
                $renamed = $log.IndexOf('Renamed application file:')
                if ($waitFinished -lt 0 -or $renamed -le $waitFinished) { throw 'Rename did not follow the WAITPID wait' }
            }

            $nextHelper = Start-LockHelper "locked-again-$binary" 180000 $binary
            try {
                Invoke-Setup $second "numbered-upgrade-$binary"
                if ($helper.HasExited -or $nextHelper.HasExited -or
                    -not (Test-Path -LiteralPath $oldPath) -or
                    -not (Test-Path -LiteralPath "$installDir/$binary.1.old")) {
                    throw 'A running .old blocked a subsequent rename-aside'
                }
                Assert-Installed "$testRoot/payload-2" '0.7.5.20260101.0002' 'https://github.com/yicheng47/runner/releases/latest'
            } finally {
                if (-not $nextHelper.HasExited) { $nextHelper.Kill(); $nextHelper.WaitForExit() }
            }
            Invoke-Setup $second "cleanup-exited-$binary"
            if (Test-Path -LiteralPath "$installDir/$binary.1.old") { throw 'Next install did not delete the exited old binary' }
            if ($helper.HasExited -or -not (Test-Path -LiteralPath $oldPath)) { throw 'Cleanup did not preserve the still-running old binary' }

            Invoke-Setup $uninstaller "uninstall-running-old-$binary"
            if ((Test-Path -LiteralPath $registryKey) -or (Test-Path -LiteralPath $shortcutPath)) { throw 'A running .old blocked uninstall' }
            foreach ($installedBinary in $binaries) {
                if (Test-Path -LiteralPath (Join-Path $installDir $installedBinary)) { throw "Uninstall left $installedBinary" }
            }
            if ($helper.HasExited -or -not (Test-Path -LiteralPath $oldPath)) { throw 'Uninstall stopped the running old helper' }
        } finally {
            if (-not $helper.HasExited) { $helper.Kill(); $helper.WaitForExit() }
            if ($waiter -and -not $waiter.HasExited) { $waiter.Kill(); $waiter.WaitForExit() }
        }
        Invoke-Setup $second "cleanup-stale-$binary" @("/DIR=`"$installDir`"")
        if (Get-ChildItem -LiteralPath $installDir -Filter '*.old') { throw 'Next install left stale old binaries after helper exit' }
    }

    $cancelHelpers = @()
    $uninstallProcess = $null
    $confirmation = [IntPtr]::Zero
    try {
        foreach ($binary in $binaries) {
            $cancelHelpers += Start-LockHelper "cancel-uninstall-$binary" 180000 $binary
        }
        $uninstallProcess = Start-Process -FilePath $uninstaller -ArgumentList @(
            '/NORESTART', "/LOG=`"$testRoot/cancel-uninstall.log`""
        ) -WindowStyle Hidden -PassThru
        for ($attempt = 0; $attempt -lt 100; $attempt++) {
            $confirmation = [InstallerTestWindow]::FindWindow('#32770', "$appName Uninstall")
            if ($confirmation -ne [IntPtr]::Zero) { break }
            Start-Sleep -Milliseconds 50
        }
        if ($confirmation -eq [IntPtr]::Zero) { throw 'Uninstall confirmation did not appear' }
        Assert-Installed "$testRoot/payload-2" '0.7.5.20260101.0002' 'https://github.com/yicheng47/runner/releases/latest'
        if (Get-ChildItem -LiteralPath $installDir -Filter '*.old') { throw 'Uninstall renamed files before confirmation' }
        [InstallerTestWindow]::SendMessage($confirmation, 0x0111, [IntPtr]7, [IntPtr]::Zero) | Out-Null
        if (-not $uninstallProcess.WaitForExit(5000)) { throw 'Uninstall did not exit after No' }
        Assert-Installed "$testRoot/payload-2" '0.7.5.20260101.0002' 'https://github.com/yicheng47/runner/releases/latest'
        foreach ($helper in $cancelHelpers) {
            if ($helper.HasExited) { throw 'Canceled uninstall stopped a running helper' }
        }
    } finally {
        if ($confirmation -ne [IntPtr]::Zero) {
            [InstallerTestWindow]::SendMessage($confirmation, 0x0111, [IntPtr]7, [IntPtr]::Zero) | Out-Null
        }
        if ($uninstallProcess -and -not $uninstallProcess.HasExited) {
            $uninstallProcess.Kill(); $uninstallProcess.WaitForExit()
        }
        foreach ($helper in $cancelHelpers) {
            if (-not $helper.HasExited) { $helper.Kill(); $helper.WaitForExit() }
        }
    }

    try {
        Invoke-Setup $second 'relaunch-success' @('/RELAUNCH=1')
    } finally {
        Stop-RelaunchedFixture
    }

    Invoke-Setup $first 'baseline-copy-failure'
    $copyHelpers = @()
    $lock = $null
    try {
        foreach ($binary in $binaries) {
            $copyHelpers += Start-LockHelper "copy-failure-$binary" 180000 $binary
        }
        $lock = [IO.File]::Open((Join-Path $installDir 'LICENSE'), [IO.FileMode]::Open, [IO.FileAccess]::Read, [IO.FileShare]::Read)
        Invoke-Setup $second 'copy-failure' @('/RELAUNCH=1') -ExpectFailure
        foreach ($helper in $copyHelpers) {
            if ($helper.HasExited) { throw 'Copy-failure helper exited before Setup finished' }
        }
        Assert-Installed "$testRoot/payload-1" '0.7.5.20260101.0001'
        foreach ($binary in $binaries) {
            if (-not (Select-String -LiteralPath "$testRoot/copy-failure.log" -SimpleMatch "Restored application file: $installDir\$binary" -Quiet)) {
                throw "Aborted install did not restore $binary"
            }
        }
    } finally {
        if ($lock) { $lock.Dispose() }
        foreach ($helper in $copyHelpers) {
            if (-not $helper.HasExited) { $helper.Kill(); $helper.WaitForExit() }
        }
        Stop-RelaunchedFixture
    }
    Invoke-Setup $second 'after-copy-failure'
    Assert-Installed "$testRoot/payload-2" '0.7.5.20260101.0002' 'https://github.com/yicheng47/runner/releases/latest'
    Write-Host 'PASS: WAITPID precedes rename, running binaries move aside, numbered old files do not block install/uninstall, canceling uninstall preserves binaries, exited old files are cleaned up, successful and failed-copy installs relaunch'
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
