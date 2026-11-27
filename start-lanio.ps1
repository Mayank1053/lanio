$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $MyInvocation.MyCommand.Path
$EnvFile = Join-Path $Root ".env"

if (-not (Test-Path $EnvFile)) {
    Write-Error ".env file not found at $EnvFile"
    exit 1
}

Get-Content $EnvFile | ForEach-Object {
    $line = $_.Trim()
    if ($line -and -not $line.StartsWith("#")) {
        $parts = $line -split "=", 2
        if ($parts.Length -eq 2) {
            [Environment]::SetEnvironmentVariable($parts[0].Trim(), $parts[1].Trim(), "Process")
        }
    }
}

$MediaPath = $env:MEDIA_PATH
if (-not $MediaPath -or -not (Test-Path $MediaPath)) {
    Write-Warning "MEDIA_PATH ('$MediaPath') does not exist. Update MEDIA_PATH in .env to your movies/TV shows folder."
    exit 1
}

$ExePath = Join-Path $Root "target\release\lanio.exe"
if (-not (Test-Path $ExePath)) {
    Write-Host "Release binary not found. Building with cargo..."
    Push-Location $Root
    cargo build --release
    Pop-Location
}

Write-Host "Starting Lanio on $env:BASE_URL serving '$MediaPath'..."
& $ExePath
