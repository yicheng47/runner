$ErrorActionPreference = 'Stop'
$probeDir = Join-Path ([IO.Path]::GetTempPath()) ('runner batch probe ' + [Guid]::NewGuid())
[IO.Directory]::CreateDirectory($probeDir) | Out-Null
try {
    $source = Join-Path $probeDir 'echo.rs'
    $exe = Join-Path $probeDir 'echo.exe'
    $batch = Join-Path $probeDir 'argument echo.bat'
    @'
fn main() {
    for (index, arg) in std::env::args().skip(1).enumerate() {
        println!("ARG_{index}={arg:?}");
    }
    println!("ENV_CLEARED={}", std::env::var_os("RUNNER_BATCH_COMMAND_LINE").is_none());
}
'@ | Set-Content -LiteralPath $source -Encoding utf8
    & rustc --edition 2024 $source -o $exe
    if ($LASTEXITCODE -ne 0) { throw 'probe compilation failed' }
    "@echo off`r`n`"$exe`" %*`r`n" | Set-Content -LiteralPath $batch -Encoding ascii

    $hackArgs = '"two words" "80%%cd:~,% coverage" "say ""hello""" "bang!" "left&right" "tail\\" "a^b>c|d"'
    $rawArgs = $hackArgs.Replace('%%cd:~,%', '%')
    $prefix = '/e:ON /v:OFF /d /c '
    $clear = 'set "RUNNER_BATCH_COMMAND_LINE=" & '
    $expected = @(
        'ARG_0="two words"',
        'ARG_1="80% coverage"',
        'ARG_2="say \"hello\""',
        'ARG_3="bang!"',
        'ARG_4="left&right"',
        'ARG_5="tail\\"',
        'ARG_6="a^b>c|d"',
        'ENV_CLEARED=true'
    )
    $cases = @()
    foreach ($variant in @(@{ Name = 'hack'; Args = $hackArgs }, @{ Name = 'raw'; Args = $rawArgs })) {
        $inner = '"' + $batch + '" ' + $variant.Args
        $outer = '"' + $inner + '"'
        $cases += @{ Name = "direct-$($variant.Name)"; Arguments = $prefix + $outer; Value = $null }
        $cases += @{ Name = "env-inner-$($variant.Name)"; Arguments = $prefix + '%RUNNER_BATCH_COMMAND_LINE%'; Value = $clear + $inner }
        $cases += @{ Name = "env-outer-$($variant.Name)"; Arguments = $prefix + '%RUNNER_BATCH_COMMAND_LINE%'; Value = $clear + $outer }
        $cases += @{ Name = "env-nested-$($variant.Name)"; Arguments = $prefix + '%RUNNER_BATCH_COMMAND_LINE%'; Value = $clear + 'cmd.exe ' + $prefix + $outer }
    }
    $passedEnv = @()
    foreach ($case in $cases) {
        $start = [Diagnostics.ProcessStartInfo]::new()
        $start.FileName = $env:ComSpec
        $start.Arguments = $case.Arguments
        $start.UseShellExecute = $false
        $start.RedirectStandardOutput = $true
        $start.RedirectStandardError = $true
        $start.CreateNoWindow = $true
        if ($null -ne $case.Value) { $start.Environment['RUNNER_BATCH_COMMAND_LINE'] = $case.Value }
        $process = [Diagnostics.Process]::Start($start)
        $stdoutTask = $process.StandardOutput.ReadToEndAsync()
        $stderrTask = $process.StandardError.ReadToEndAsync()
        if (-not $process.WaitForExit(5000)) {
            $process.Kill($true)
            throw "probe timed out: $($case.Name)"
        }
        $stdout = $stdoutTask.GetAwaiter().GetResult()
        $stderr = $stderrTask.GetAwaiter().GetResult()
        $ok = $process.ExitCode -eq 0 -and -not $stdout.Contains('ARG_7=')
        foreach ($line in $expected) { $ok = $ok -and $stdout.Contains($line) }
        [ordered]@{ case = $case.Name; passed = $ok; exit = $process.ExitCode; stdout = $stdout; stderr = $stderr } | ConvertTo-Json -Compress
        if ($ok -and $case.Name.StartsWith('env-')) { $passedEnv += $case.Name }
        $process.Dispose()
    }
    Write-Output ('PASSING_ENV_VARIANTS=' + ($passedEnv -join ','))
    if ($passedEnv.Count -eq 0) { throw 'No environment route preserved all arguments and cleared its variable' }
} finally {
    [IO.Directory]::Delete($probeDir, $true)
}
