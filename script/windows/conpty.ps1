param([Parameter(Mandatory = $true)][string]$OutputDir)

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
# make.cmd can inherit PowerShell 7's module path before starting Windows PowerShell.
Import-Module (Join-Path $PSHOME 'Modules/Microsoft.PowerShell.Utility')
Import-Module (Join-Path $PSHOME 'Modules/Microsoft.PowerShell.Security')
$repo = Split-Path (Split-Path $PSScriptRoot -Parent) -Parent
$version = '1.24.260303001'
$sha256 = '2c57cb7da7e19fa06c86487c8d9b5c307d65695429fa15a854bf5f3cddca9e1d'
$download = Join-Path $repo "target/tools/Microsoft.Windows.Console.ConPTY.$version.nupkg"
$toolDir = Join-Path $repo "target/tools/conpty-$version"
if (-not (Test-Path -LiteralPath (Join-Path $toolDir 'conpty.dll')) -or
    -not (Test-Path -LiteralPath (Join-Path $toolDir 'OpenConsole.exe'))) {
    New-Item -ItemType Directory -Force -Path $toolDir | Out-Null
    $package = $download
    if (-not (Test-Path -LiteralPath $download)) {
        $package = "$download.partial"
        Write-Host "Downloading ConPTY $version..."
        Invoke-WebRequest "https://github.com/microsoft/terminal/releases/download/v1.24.10621.0/Microsoft.Windows.Console.ConPTY.$version.nupkg" -OutFile $package
    }
    if ((Get-FileHash -LiteralPath $package -Algorithm SHA256).Hash -ne $sha256) {
        throw "ConPTY checksum mismatch: $package"
    }
    if ($package -ne $download) {
        Move-Item -LiteralPath $package -Destination $download
    }

    Add-Type -AssemblyName System.IO.Compression.FileSystem
    $archive = [IO.Compression.ZipFile]::OpenRead($download)
    try {
        foreach ($path in 'runtimes/win-x64/native/conpty.dll', 'build/native/runtimes/x64/OpenConsole.exe') {
            $destination = Join-Path $toolDir ([IO.Path]::GetFileName($path))
            [IO.Compression.ZipFileExtensions]::ExtractToFile($archive.GetEntry($path), $destination, $true)
            $signature = Get-AuthenticodeSignature -LiteralPath $destination
            if ($signature.Status -ne 'Valid' -or $signature.SignerCertificate.Subject -notmatch '(^|,\s*)O=Microsoft Corporation(,|$)') {
                Remove-Item -LiteralPath $destination
                throw "ConPTY file does not have a valid Microsoft signature: $destination"
            }
        }
    } finally {
        $archive.Dispose()
    }
}

New-Item -ItemType Directory -Force -Path $OutputDir | Out-Null
foreach ($binary in 'conpty.dll', 'OpenConsole.exe') {
    Copy-Item -LiteralPath (Join-Path $toolDir $binary) -Destination (Join-Path $OutputDir $binary) -Force
}
