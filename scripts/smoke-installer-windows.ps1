param([Parameter(Mandatory)][string]$Installer)
$ErrorActionPreference = 'Stop'
$testRoot = Join-Path ([IO.Path]::GetTempPath()) ('stutter-watch-installer-' + [guid]::NewGuid().ToString('N'))
$directory = Join-Path $testRoot 'app'
New-Item -ItemType Directory -Path $testRoot | Out-Null
function Invoke-Checked([string]$File, [string[]]$Arguments) {
    $p = Start-Process -FilePath $File -ArgumentList $Arguments -PassThru -WindowStyle Hidden
    if (-not $p.WaitForExit(90000)) { throw "Timed out: $File" }
    if ($p.ExitCode -ne 0) { throw "Failed with exit $($p.ExitCode): $File" }
}
function Wait-Recorder([string]$Config) {
    $deadline = [DateTime]::UtcNow.AddSeconds(20)
    do {
        $endpoint = Join-Path (Split-Path $Config) 'config.toml.instance.json'
        if (Test-Path -LiteralPath $endpoint) {
            try {
                $instance = Get-Content -LiteralPath $endpoint -Raw | ConvertFrom-Json
                if (Get-Process -Id $instance.pid -ErrorAction SilentlyContinue) { return $instance.pid }
            } catch { }
        }
        Start-Sleep -Milliseconds 100
    } while ([DateTime]::UtcNow -lt $deadline)
    throw 'Recorder did not start'
}
$installArgs = @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART','/CURRENTUSER','/NOICONS',('/DIR="' + $directory + '"'))
Invoke-Checked $Installer $installArgs
$exe = Join-Path $directory 'stutter-watch.exe'
if (-not (Test-Path -LiteralPath $exe)) { throw 'Installer did not create the executable' }
$config = Join-Path $directory 'config.toml'
Invoke-Checked $exe @('--config',('"' + $config + '"'),'status')
$configHash = (Get-FileHash -LiteralPath $config).Hash
$protected = Join-Path $directory 'logs/protected/keep-user-data.txt'
New-Item -ItemType Directory -Path (Split-Path $protected) -Force | Out-Null
[IO.File]::WriteAllText($protected, 'Synthetic user data: preserve across update and uninstall')
$dataHash = (Get-FileHash -LiteralPath $protected).Hash
$p = Start-Process -FilePath $exe -ArgumentList @('--config',('"' + $config + '"'),'--no-tray','run') -PassThru -WindowStyle Hidden
$oldPid = Wait-Recorder $config
Start-Sleep -Seconds 3
Invoke-Checked $Installer $installArgs
if (Get-Process -Id $oldPid -ErrorAction SilentlyContinue) { throw 'Update left the old recorder running' }
$newPid = Wait-Recorder $config
if ((Get-FileHash -LiteralPath $config).Hash -ne $configHash) { throw 'Update changed user config' }
if ((Get-FileHash -LiteralPath $protected).Hash -ne $dataHash) { throw 'Update changed user data' }
# Exercise rollback independently with a deliberately unusable replacement.
$helper = Join-Path $testRoot 'maintenance.exe'
Copy-Item -LiteralPath $exe -Destination $helper
$receipt = Join-Path $testRoot 'rollback.json'
$lifecycleArgs = @('--directory',('"' + $directory + '"'),'--receipt',('"' + $receipt + '"'))
$original = (Get-FileHash -LiteralPath $exe).Hash
Invoke-Checked $helper (@('maintenance','prepare') + $lifecycleArgs)
[IO.File]::WriteAllBytes($exe, [byte[]](0,1,2,3))
Invoke-Checked $helper (@('maintenance','rollback') + $lifecycleArgs)
if ((Get-FileHash -LiteralPath $exe).Hash -ne $original) { throw 'Rollback did not restore executable' }
$null = Wait-Recorder $config
Invoke-Checked (Join-Path $directory 'unins000.exe') @('/VERYSILENT','/SUPPRESSMSGBOXES','/NORESTART')
if (Test-Path -LiteralPath $exe) { throw 'Uninstaller left executable behind' }
if ((Get-FileHash -LiteralPath $config).Hash -ne $configHash) { throw 'Uninstall changed user config' }
if ((Get-FileHash -LiteralPath $protected).Hash -ne $dataHash) { throw 'Uninstall changed user data' }
@{ fresh_install=$true; running_update=$true; rollback=$true; uninstall_keeps_data=$true } | ConvertTo-Json
