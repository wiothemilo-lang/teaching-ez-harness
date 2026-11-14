# Cài tez trên Windows từ GitHub Releases.
#
#   powershell -ExecutionPolicy Bypass -File scripts\windows\install.ps1
#   powershell -ExecutionPolicy Bypass -File scripts\windows\install.ps1 -Version 0.2.0
#
# Mặc định cài vào %LOCALAPPDATA%\tez, tạo shortcut Start Menu và thêm vào PATH.

param(
    [string]$Version = "latest",
    [string]$Repo = $(if ($env:TEZ_GITHUB_REPO) { $env:TEZ_GITHUB_REPO } else { "teaching-ez/tez" }),
    [switch]$NoShortcut,
    [switch]$NoPath
)

$ErrorActionPreference = "Stop"
$InstallDir = Join-Path $env:LOCALAPPDATA "tez"
$ApiUrl = if ($Version -eq "latest") {
    "https://api.github.com/repos/$Repo/releases/latest"
} else {
    "https://api.github.com/repos/$Repo/releases/tags/v$($Version.TrimStart('v'))"
}

Write-Host "tez installer · repo $Repo · $Version" -ForegroundColor Cyan

$release = Invoke-RestMethod -Uri $ApiUrl -Headers @{
    "User-Agent" = "tez-installer"
    "Accept"     = "application/vnd.github+json"
}

$asset = $release.assets |
    Where-Object { $_.name -match "(x64|x86_64|amd64).*(setup\.exe|\.msi|\.zip)$" -or $_.name -match "(setup\.exe|\.msi|\.zip)$" } |
    Select-Object -First 1

if (-not $asset) {
    throw "Không tìm thấy file cài Windows trong release $($release.tag_name). Mở: $($release.html_url)"
}

Write-Host "Tải $($asset.name)…" -ForegroundColor Cyan
$temp = Join-Path $env:TEMP $asset.name
Invoke-WebRequest -Uri $asset.browser_download_url -OutFile $temp -Headers @{ "User-Agent" = "tez-installer" }

if ($asset.name -like "*.msi") {
    Write-Host "Chạy installer MSI…" -ForegroundColor Cyan
    Start-Process msiexec.exe -ArgumentList "/i", "`"$temp`"", "/passive" -Wait
    exit 0
}

if ($asset.name -like "*setup.exe") {
    Write-Host "Chạy installer EXE…" -ForegroundColor Cyan
    Start-Process -FilePath $temp -Wait
    exit 0
}

# Portable zip: giải nén vào %LOCALAPPDATA%\tez
New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$extract = Join-Path $env:TEMP "tez-extract"
if (Test-Path $extract) { Remove-Item $extract -Recurse -Force }
Expand-Archive -Path $temp -DestinationPath $extract -Force

$exe = Get-ChildItem -Path $extract -Recurse -Filter "tez*.exe" |
    Where-Object { $_.Name -notlike "*cli*" } |
    Select-Object -First 1
if (-not $exe) {
    $exe = Get-ChildItem -Path $extract -Recurse -Filter "*.exe" | Select-Object -First 1
}
if (-not $exe) { throw "Không thấy file .exe trong gói tải về." }

Copy-Item $exe.FullName (Join-Path $InstallDir "tez.exe") -Force
Get-ChildItem -Path $exe.Directory -File | Where-Object { $_.Extension -ne ".exe" } |
    Copy-Item -Destination $InstallDir -Force -ErrorAction SilentlyContinue

if (-not $NoShortcut) {
    $startMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
    $shortcut = Join-Path $startMenu "tez.lnk"
    $shell = New-Object -ComObject WScript.Shell
    $link = $shell.CreateShortcut($shortcut)
    $link.TargetPath = Join-Path $InstallDir "tez.exe"
    $link.WorkingDirectory = $InstallDir
    $link.Description = "tez — Chat & Code"
    $link.Save()
    Write-Host "Đã tạo shortcut: $shortcut" -ForegroundColor Green
}

if (-not $NoPath) {
    $userPath = [Environment]::GetEnvironmentVariable("Path", "User")
    if ($userPath -notlike "*$InstallDir*") {
        [Environment]::SetEnvironmentVariable("Path", "$userPath;$InstallDir", "User")
        Write-Host "Đã thêm $InstallDir vào PATH (mở terminal mới để dùng)." -ForegroundColor Green
    }
}

Write-Host "Cài xong tez $($release.tag_name) vào $InstallDir" -ForegroundColor Green
Write-Host "Mở bằng: $InstallDir\tez.exe"
