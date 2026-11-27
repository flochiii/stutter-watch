param([switch]$NoPanel)
$ErrorActionPreference = 'Stop'
$workspace = Split-Path -Parent $PSScriptRoot
$executable = Join-Path $workspace 'target\release\stutter-watch.exe'
if (-not (Test-Path -LiteralPath $executable)) { throw 'Build first: cargo build --locked --release' }
$testRoot = Join-Path $workspace '.verification\dev'
New-Item -ItemType Directory -Path $testRoot -Force | Out-Null
$configPath = Join-Path $testRoot 'config.toml'
if (-not (Test-Path -LiteralPath $configPath)) {
    $text = [IO.File]::ReadAllText((Join-Path $workspace 'config.example.toml')).Replace('Ctrl+Shift+F12', 'Ctrl+Alt+Shift+F11')
    [IO.File]::WriteAllText($configPath, $text, [Text.UTF8Encoding]::new($false))
}
$launchArgs = @('--config', ('"' + $configPath + '"'))
# The GUI subsystem allocates no console. Show is sent separately so the
# development helper's hidden startup flag does not hide the interactive panel.
Start-Process -FilePath $executable -ArgumentList $launchArgs -WorkingDirectory $workspace -WindowStyle Hidden
if (-not $NoPanel) {
    $endpoint = $configPath + '.instance.json'
    $deadline = [DateTime]::UtcNow.AddSeconds(10)
    while (-not (Test-Path -LiteralPath $endpoint) -and [DateTime]::UtcNow -lt $deadline) {
        Start-Sleep -Milliseconds 100
    }
    & $executable --config $configPath show
    if ($LASTEXITCODE -ne 0) { throw 'The development recorder did not become ready. Check its startup diagnostics.' }
}
Write-Output "Development config: $configPath"
Write-Output 'Bookmark: Ctrl+Alt+Shift+F11. Quit using the development recorder tray menu when finished.'
