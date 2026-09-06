$ErrorActionPreference = 'Stop'

$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$toolDir = Join-Path $repo 'target/tools/inno-setup-6.7.3'
$compiler = Join-Path $toolDir 'ISCC.exe'
if (-not (Test-Path -LiteralPath $compiler)) {
    $download = Join-Path $repo 'target/tools/innosetup-6.7.3.exe'
    New-Item -ItemType Directory -Force -Path (Split-Path $download -Parent) | Out-Null
    $sha256 = '9c73c3bae7ed48d44112a0f48e66742c00090bdb5bef71d9d3c056c66e97b732'
    if (-not (Test-Path -LiteralPath $download)) {
        Write-Host 'Downloading Inno Setup 6.7.3...'
        Invoke-WebRequest 'https://github.com/jrsoftware/issrc/releases/download/is-6_7_3/innosetup-6.7.3.exe' -OutFile $download
    }
    if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ne $sha256) {
        throw "Inno Setup checksum mismatch: $download"
    }
    $setup = Start-Process -FilePath $download -ArgumentList @(
        '/VERYSILENT', '/SUPPRESSMSGBOXES', '/NORESTART', '/CURRENTUSER', '/PORTABLE=1', "/DIR=`"$toolDir`""
    ) -WindowStyle Hidden -Wait -PassThru
    if ($setup.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $compiler)) {
        throw "Inno Setup compiler extraction failed: $($setup.ExitCode)"
    }
}
$compiler
