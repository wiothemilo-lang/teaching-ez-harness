# Build tez trên Windows (dev machine).
#
#   powershell -ExecutionPolicy Bypass -File scripts\windows\build.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\windows\build.ps1 -Debug

param(
    [switch]$Debug
)

$ErrorActionPreference = "Stop"
$RepoRoot = Resolve-Path (Join-Path $PSScriptRoot "..\..")

Write-Host "tez build · Windows" -ForegroundColor Cyan

if (-not (Get-Command cargo -ErrorAction SilentlyContinue)) {
    throw "Chưa có Rust. Cài từ https://rustup.rs rồi mở terminal mới."
}
if (-not (Get-Command pnpm -ErrorAction SilentlyContinue)) {
    Write-Host "Cài pnpm…" -ForegroundColor Yellow
    npm install -g pnpm
}

Push-Location $RepoRoot
try {
    $profileArgs = if ($Debug) { @() } else { @("--release") }
    Write-Host "Build CLI…" -ForegroundColor Cyan
    cargo build @profileArgs -p tez-cli

    Write-Host "Build desktop…" -ForegroundColor Cyan
    Push-Location (Join-Path $RepoRoot "apps\desktop")
    try {
        pnpm install
        if ($Debug) {
            pnpm tauri build --debug
        } else {
            pnpm tauri build
        }
    } finally {
        Pop-Location
    }
} finally {
    Pop-Location
}

$bundle = Join-Path $RepoRoot "apps\desktop\src-tauri\target\release\bundle"
Write-Host "Xong. Bundle (nếu có): $bundle" -ForegroundColor Green
