# tez launcher for Windows.
# Chạy bản đã cài, bản build local, hoặc dev mode.
#
#   .\tez.ps1            mở app
#   .\tez.ps1 --dev      chạy pnpm tauri dev
#   .\tez.ps1 --cli ...  chạy CLI tez

param(
    [Parameter(ValueFromRemainingArguments = $true)]
    [string[]]$Rest
)

$ErrorActionPreference = "Stop"
$DevMode = $Rest -contains "--dev"
$CliMode = $Rest -contains "--cli"
$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")
$CliArgs = @($Rest | Where-Object { $_ -ne "--dev" -and $_ -ne "--cli" })

function Find-DesktopExe {
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA "tez\tez.exe"),
        (Join-Path $env:LOCALAPPDATA "Programs\tez\tez.exe"),
        (Join-Path $RepoRoot "target\release\tez-desktop.exe"),
        (Join-Path $RepoRoot "target\debug\tez-desktop.exe"),
        (Join-Path $RepoRoot "apps\desktop\src-tauri\target\release\tez-desktop.exe"),
        (Join-Path $RepoRoot "apps\desktop\src-tauri\target\debug\tez-desktop.exe")
    )
    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) { return $candidate }
    }
    return $null
}

function Find-CliExe {
    $candidates = @(
        (Join-Path $env:LOCALAPPDATA "tez\tez-cli.exe"),
        (Join-Path $RepoRoot "target\release\tez.exe"),
        (Join-Path $RepoRoot "target\debug\tez.exe")
    )
    foreach ($candidate in $candidates) {
        if (Test-Path $candidate) { return $candidate }
    }
    return $null
}

if ($CliMode) {
    $cli = Find-CliExe
    if ($cli) {
        & $cli @CliArgs
        exit $LASTEXITCODE
    }
    Write-Host "Chưa có tez CLI. Build bằng: cargo build --release -p tez-cli" -ForegroundColor Yellow
    exit 1
}

if ($DevMode) {
    if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
        Write-Host "Cần pnpm để chạy dev mode: npm install -g pnpm" -ForegroundColor Red
        exit 1
    }
    Push-Location (Join-Path $RepoRoot "apps\desktop")
    try {
        pnpm install
        pnpm tauri dev
    } finally {
        Pop-Location
    }
    exit $LASTEXITCODE
}

$exe = Find-DesktopExe
if (-not $exe) {
    Write-Host "Chưa tìm thấy tez trên máy." -ForegroundColor Yellow
    Write-Host "Cài bản phát hành:  powershell -ExecutionPolicy Bypass -File scripts\windows\install.ps1"
    Write-Host "Hoặc chạy dev:       scripts\windows\tez.bat --dev"
    exit 1
}

Start-Process -FilePath $exe -WorkingDirectory (Split-Path $exe)
