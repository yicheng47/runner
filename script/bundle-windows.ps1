param(
    [ValidateSet('nightly', 'production')]
    [string]$Channel = 'nightly',
    [ValidatePattern('^\d{8}\.\d{4}$')]
    [string]$Stamp = $env:RUNNER_BUILD_STAMP,
    [string]$Sha = $env:RUNNER_BUILD_SHA,
    [ValidateRange(1, 256)]
    [int]$Jobs = [Math]::Min(12, [Environment]::ProcessorCount)
)

$ErrorActionPreference = 'Stop'
$repo = Split-Path $PSScriptRoot -Parent
if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    $cargoHome = if ($env:CARGO_HOME) { $env:CARGO_HOME } else { Join-Path $env:USERPROFILE '.cargo' }
    $env:Path = (Join-Path $cargoHome 'bin') + ';' + $env:Path
}
$compiler = & (Join-Path $PSScriptRoot 'windows/inno-setup.ps1')
$previousStamp = $env:RUNNER_BUILD_STAMP
$previousSha = $env:RUNNER_BUILD_SHA
$previousMarketing = $env:RUNNER_MARKETING_VERSION
$previousChannel = $env:RUNNER_RELEASE_CHANNEL
$previousRustflags = $env:RUSTFLAGS
Push-Location $repo
try {
    if (-not $Stamp) { $Stamp = [DateTime]::UtcNow.ToString('yyyyMMdd.HHmm') }
    if (-not $Sha) {
        $Sha = git rev-parse --short=7 HEAD
        if ($LASTEXITCODE -ne 0) { throw 'Cannot determine the build commit' }
    }
    $metadataJson = cargo metadata --locked --format-version 1 --no-deps
    if ($LASTEXITCODE -ne 0) { throw 'Cargo metadata failed' }
    $metadata = $metadataJson | ConvertFrom-Json
    $version = ($metadata.packages | Where-Object name -eq 'runner-app').version
    $baseVersion = $version -replace '-.*$', ''
    $shortVersion = if ($Channel -eq 'production') { "$baseVersion.$Stamp" } else { "$version.$Stamp" }
    $updatesUrl = if ($Channel -eq 'production') {
        'https://github.com/yicheng47/runner/releases/latest'
    } else {
        'https://github.com/yicheng47/runner/releases/tag/nightly-win'
    }
    $env:RUNNER_BUILD_STAMP = $Stamp
    $env:RUNNER_BUILD_SHA = $Sha
    $env:RUNNER_MARKETING_VERSION = if ($Channel -eq 'production') { $baseVersion } else { $shortVersion }
    $env:RUNNER_RELEASE_CHANNEL = $Channel
    $env:RUSTFLAGS = "$previousRustflags -C target-feature=+crt-static".Trim()
    cargo build --locked --release -p runner-app -p runner-cli --target x86_64-pc-windows-msvc -j $Jobs
    if ($LASTEXITCODE -ne 0) { throw 'Windows release build failed' }

    $release = Join-Path $metadata.target_directory 'x86_64-pc-windows-msvc/release'
    $vswhere = Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    $dumpbin = & $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -find 'VC/Tools/MSVC/**/bin/Hostx64/x64/dumpbin.exe' | Select-Object -First 1
    if (-not $dumpbin) { throw 'Cannot find dumpbin in the MSVC build tools' }
    foreach ($binary in 'Runner.exe', 'runner-agent-cli.exe', 'runner-mcp.exe') {
        $binaryPath = Join-Path $release $binary
        $imports = & $dumpbin /dependents $binaryPath
        if ($LASTEXITCODE -ne 0) { throw "Cannot inspect $binary dependencies" }
        if ($imports -match '(?i)\b(vcruntime|msvcp|concrt)\d+.*\.dll') {
            throw "$binary requires a Visual C++ runtime DLL; check the static runtime build flags"
        }
    }
    $headers = & $dumpbin /headers (Join-Path $release 'Runner.exe')
    if ($LASTEXITCODE -ne 0 -or -not ($headers -match '^\s+2 subsystem')) {
        throw 'Runner.exe must use the Windows GUI subsystem'
    }
    & $compiler /Qp "/DAppVersion=$shortVersion" "/DBaseVersion=$baseVersion" "/DUpdatesUrl=$updatesUrl" "/DSourceDir=$release" "/DOutputDir=$release" (Join-Path $PSScriptRoot 'windows/runner.iss')
    if ($LASTEXITCODE -ne 0) { throw 'Windows installer compilation failed' }

    Write-Host "Unsigned installer: $(Join-Path $release "Runner-Setup-$shortVersion-x64.exe")"
} finally {
    $env:RUNNER_BUILD_STAMP = $previousStamp
    $env:RUNNER_BUILD_SHA = $previousSha
    $env:RUNNER_MARKETING_VERSION = $previousMarketing
    $env:RUNNER_RELEASE_CHANNEL = $previousChannel
    $env:RUSTFLAGS = $previousRustflags
    Pop-Location
}
