param(
    [string]$UserName = $env:CERTUM_USERNAME,
    [string]$OtpUri = $env:CERTUM_OTP_URI,
    [string]$Thumbprint = $env:CERTUM_CERTIFICATE_SHA1
)

# Connects a CI runner to Certum SimplySign so signtool can use the cloud-held
# code-signing key. SimplySign Desktop has no headless mode: it must be logged
# in through its dialog with the account name and a one-time code, after which
# the certificate appears in the user store backed by a virtual smart card for
# about two hours.

$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
if (-not ($UserName -and $OtpUri -and $Thumbprint)) {
    throw 'CERTUM_USERNAME, CERTUM_OTP_URI, and CERTUM_CERTIFICATE_SHA1 are required'
}

$version = '9.4.4.92'
$sha256 = '8ec420fc27798b86078b7bd02fe7152097e1b3005bab51820eaca8e57df84da3'
$exe = Join-Path $env:ProgramFiles 'Certum/SimplySign Desktop/SimplySignDesktop.exe'
if (-not (Test-Path -LiteralPath $exe)) {
    $download = Join-Path ([IO.Path]::GetTempPath()) "SimplySignDesktop-$version-64-bit-en.msi"
    Write-Host "Downloading SimplySign Desktop $version..."
    Invoke-WebRequest "https://files.certum.eu/software/SimplySignDesktop/Windows/$version/SimplySignDesktop-$version-64-bit-en.msi" -OutFile $download
    if ((Get-FileHash -LiteralPath $download -Algorithm SHA256).Hash -ne $sha256) {
        throw "SimplySign Desktop checksum mismatch: $download"
    }
    $install = Start-Process -FilePath msiexec.exe -ArgumentList @('/i', "`"$download`"", '/qn', '/norestart') -Wait -PassThru
    if ($install.ExitCode -ne 0 -or -not (Test-Path -LiteralPath $exe)) {
        throw "SimplySign Desktop installation failed: $($install.ExitCode)"
    }
}

Add-Type -AssemblyName System.Web
$query = [Web.HttpUtility]::ParseQueryString(([Uri]$OtpUri).Query)
$secret = $query['secret']
if (-not $secret) { throw 'CERTUM_OTP_URI has no secret parameter' }
$algorithm = if ($query['algorithm']) { $query['algorithm'].ToUpperInvariant() } else { 'SHA1' }
$digits = if ($query['digits']) { [int]$query['digits'] } else { 6 }
$period = if ($query['period']) { [int]$query['period'] } else { 30 }

$alphabet = 'ABCDEFGHIJKLMNOPQRSTUVWXYZ234567'
$key = [Collections.Generic.List[byte]]::new()
$buffer = [long]0
$bufferBits = 0
foreach ($char in $secret.TrimEnd('=').ToUpperInvariant().ToCharArray()) {
    $value = $alphabet.IndexOf($char)
    if ($value -lt 0) { throw 'CERTUM_OTP_URI secret is not Base32' }
    $buffer = (($buffer -shl 5) -bor $value) -band 0xFFFF
    $bufferBits += 5
    if ($bufferBits -ge 8) {
        $key.Add([byte](($buffer -shr ($bufferBits - 8)) -band 0xFF))
        $bufferBits -= 8
    }
}
$keyBytes = $key.ToArray()

function Get-OneTimeCode {
    $counter = [BitConverter]::GetBytes([long][Math]::Floor([DateTimeOffset]::UtcNow.ToUnixTimeSeconds() / $period))
    if ([BitConverter]::IsLittleEndian) { [Array]::Reverse($counter) }
    $hmac = switch ($algorithm) {
        'SHA1' { [Security.Cryptography.HMACSHA1]::new($keyBytes) }
        'SHA256' { [Security.Cryptography.HMACSHA256]::new($keyBytes) }
        'SHA512' { [Security.Cryptography.HMACSHA512]::new($keyBytes) }
        default { throw "Unsupported one-time code algorithm: $algorithm" }
    }
    try {
        $hash = $hmac.ComputeHash($counter)
    } finally {
        $hmac.Dispose()
    }
    $offset = $hash[-1] -band 0x0F
    $binary = (([long]$hash[$offset] -band 0x7F) -shl 24) -bor ([long]$hash[$offset + 1] -shl 16) -bor ([long]$hash[$offset + 2] -shl 8) -bor [long]$hash[$offset + 3]
    ($binary % [long][Math]::Pow(10, $digits)).ToString().PadLeft($digits, '0')
}

function Test-Connected {
    $certificate = Get-ChildItem -Path Cert:\CurrentUser\My | Where-Object Thumbprint -eq $Thumbprint
    [bool]($certificate -and $certificate.HasPrivateKey)
}

function Send-Text([object]$Shell, [string]$Text) {
    $Shell.SendKeys(($Text -replace '([+^%~(){}\[\]])', '{$1}'))
}

if (Test-Connected) {
    Write-Host 'SimplySign is already connected'
    return
}
$settings = 'HKCU:\Software\Certum\SimplySign'
New-Item -Path $settings -Force | Out-Null
foreach ($entry in @{
    ShowLoginDialogOnStart = 1
    ShowLoginDialogOnAppRequest = 1
    RememberLastUserName = 0
    Autostart = 0
    UnregisterCertificatesOnDisconnect = 0
    RememberPINinCSP = 1
    ForgetPINinCSPonDisconnect = 1
    LangID = 9
}.GetEnumerator()) {
    Set-ItemProperty -Path $settings -Name $entry.Key -Value $entry.Value -Type DWord
}

$shell = New-Object -ComObject WScript.Shell
$lastCode = ''
foreach ($attempt in 1..3) {
    Get-Process -Name SimplySignDesktop -ErrorAction SilentlyContinue | Stop-Process -Force
    $code = Get-OneTimeCode
    while ($code -eq $lastCode) {
        Start-Sleep -Seconds 5
        $code = Get-OneTimeCode
    }
    $lastCode = $code
    $process = Start-Process -FilePath $exe -PassThru
    $focused = $false
    for ($wait = 0; $wait -lt 60 -and -not $focused; $wait++) {
        Start-Sleep -Milliseconds 500
        $focused = $shell.AppActivate($process.Id)
    }
    if (-not $focused) {
        Write-Host "Attempt ${attempt}: the SimplySign login dialog did not appear"
        continue
    }
    Start-Sleep -Milliseconds 500
    Send-Text $shell $UserName
    Start-Sleep -Milliseconds 200
    $shell.SendKeys('{TAB}')
    Start-Sleep -Milliseconds 200
    Send-Text $shell $code
    Start-Sleep -Milliseconds 200
    $shell.SendKeys('{ENTER}')
    for ($wait = 0; $wait -lt 60; $wait++) {
        Start-Sleep -Seconds 1
        if (Test-Connected) {
            Write-Host "SimplySign connected; certificate $Thumbprint is available for signing"
            return
        }
    }
    Write-Host "Attempt ${attempt}: certificate $Thumbprint did not appear after login"
}
throw 'Could not connect to SimplySign'
