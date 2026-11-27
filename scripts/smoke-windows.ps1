param([Parameter(Mandatory)][string]$Archive)
$ErrorActionPreference = 'Stop'
$testRoot = Join-Path ([System.IO.Path]::GetTempPath()) ('stutter-watch-smoke-' + [guid]::NewGuid().ToString('N'))
New-Item -ItemType Directory -Path $testRoot | Out-Null
Expand-Archive -LiteralPath $Archive -DestinationPath $testRoot
$executable = @(Get-ChildItem -LiteralPath $testRoot -Filter stutter-watch.exe -Recurse)
if ($executable.Count -ne 1) { throw 'Expected exactly one packaged executable' }
$build = Get-Content -LiteralPath (Join-Path $executable[0].DirectoryName 'BUILD.json') -Raw | ConvertFrom-Json
if ((Get-FileHash -LiteralPath $executable[0].FullName -Algorithm SHA256).Hash.ToLowerInvariant() -ne $build.exe_sha256) { throw 'Packaged executable hash mismatch' }
$testConfig = Join-Path $testRoot 'isolated/config.toml'
$process = Start-Process -FilePath $executable[0].FullName -ArgumentList @('--config', ('"' + $testConfig + '"'), '--self-test', '20', '--no-tray') -PassThru -WindowStyle Hidden
if (-not $process.WaitForExit(90000)) {
    Stop-Process -Id $process.Id -ErrorAction SilentlyContinue
    throw 'Isolated smoke test exceeded 90 seconds'
}
if ($process.ExitCode -ne 0) { throw "Isolated smoke test failed: $($process.ExitCode)" }
$summary = Get-Content -LiteralPath (Join-Path $testRoot 'isolated/logs/self-test-last.json') -Raw | ConvertFrom-Json
if ($summary.samples -lt 2 -or $summary.malformed_records -ne 0 -or $summary.omitted_jobs -ne 0 -or $summary.recording_error) { throw 'Smoke test reported missing or invalid data' }
$summary | Select-Object samples, records_validated, malformed_records, omitted_jobs, recording_error | ConvertTo-Json
Write-Output "Smoke test output: $testRoot"
