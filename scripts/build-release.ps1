# Builds the TradeLens release exe with the OAuth client ids from .env embedded
# (option_env!), into target-release\release, and puts the window icon next to it.
# Usage (from the repo root): powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1
$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSScriptRoot
$envFile = Join-Path $root '.env'

if (Test-Path $envFile) {
    foreach ($line in Get-Content $envFile) {
        if ($line -match '^\s*([A-Z0-9_]+)\s*=\s*(.*?)\s*$' -and $Matches[2] -ne '') {
            Set-Item -Path "env:$($Matches[1])" -Value $Matches[2]
        }
    }
}
foreach ($name in 'TRADELENS_GOOGLE_CLIENT_ID', 'TRADELENS_GOOGLE_CLIENT_SECRET', 'TRADELENS_MICROSOFT_CLIENT_ID') {
    $state = if ([Environment]::GetEnvironmentVariable($name)) { 'set' } else { 'missing' }
    Write-Host "${name}: $state"
}

$targetDir = Join-Path $root 'target-release'
cargo build --release -p nsh-app-tradelens --target-dir $targetDir
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

$out = Join-Path $targetDir 'release'
New-Item -ItemType Directory -Force (Join-Path $out 'nsh') | Out-Null
Copy-Item (Join-Path $root 'app\icon\icon.ico') (Join-Path $out 'nsh\icon.ico') -Force
Write-Host "Built: $(Join-Path $out 'nsh-app-tradelens.exe')"
