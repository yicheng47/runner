$ErrorActionPreference = 'Stop'

$kits = Join-Path ${env:ProgramFiles(x86)} 'Windows Kits/10/bin'
$signtool = Get-ChildItem -LiteralPath $kits -Directory -ErrorAction SilentlyContinue |
    Where-Object { $_.Name -match '^10\.\d+\.\d+\.\d+$' } |
    Sort-Object { [Version]$_.Name } -Descending |
    ForEach-Object { Join-Path $_.FullName 'x64/signtool.exe' } |
    Where-Object { Test-Path -LiteralPath $_ } |
    Select-Object -First 1
if (-not $signtool) { throw 'Cannot find signtool.exe in the Windows SDK' }
$signtool
