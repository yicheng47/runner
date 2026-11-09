param(
    [Parameter(Mandatory = $true)][string]$Uri,
    [Parameter(Mandatory = $true)][string]$OutFile
)

$ErrorActionPreference = 'Stop'

# The same transient failures and bounds as curl's --retry 5 --retry-delay 10.
# Windows PowerShell 5.1, which make.cmd runs, has no -MaximumRetryCount.
$retries = 5
for ($attempt = 0; ; $attempt++) {
    try {
        Invoke-WebRequest $Uri -OutFile $OutFile
        return
    } catch {
        $response = $_.Exception.Response
        $transient = (-not $response) -or ([int]$response.StatusCode -in 408, 429, 500, 502, 503, 504)
        if (-not $transient -or $attempt -ge $retries) {
            throw
        }
        Write-Host "Download failed: $($_.Exception.Message) Retrying in 10 seconds..."
        Start-Sleep -Seconds 10
    }
}
