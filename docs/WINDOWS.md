# tez trên Windows

tez có hai phần: **Chat** (tổng quát, không giới hạn môn học) và **Code** (agent đọc/sửa file, chạy lệnh trong workspace). Bản Windows dùng cùng mã nguồn Tauri 2 với Linux.

## 1. Cài bản phát hành

```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows\install.ps1
```

Script sẽ:

1. Gọi GitHub Releases (`teaching-ez/tez` hoặc `TEZ_GITHUB_REPO`).
2. Chọn file `.msi`, `setup.exe` hoặc `.zip` phù hợp.
3. Cài vào `%LOCALAPPDATA%\tez` (bản zip), tạo shortcut Start Menu và thêm vào PATH.

Tùy chọn:

```powershell
# Ghép bản cụ thể
powershell -ExecutionPolicy Bypass -File scripts\windows\install.ps1 -Version 0.2.0

# Repo khác
$env:TEZ_GITHUB_REPO = "to-chuc/tez"
powershell -ExecutionPolicy Bypass -File scripts\windows\install.ps1
```

## 2. Chạy

```bat
scripts\windows\tez.bat            :: mở app
scripts\windows\tez.bat --dev      :: chạy dev mode (pnpm tauri dev)
scripts\windows\tez.bat --cli ...  :: chạy CLI tez
```

Sau khi cài, có thể mở **tez** từ Start Menu hoặc gõ `tez` trong terminal mới.

## 3. Build từ source

Yêu cầu:

- Rust stable (`rustup`) + MSVC Build Tools
- Node 20+ và pnpm
- WebView2 (Windows 10/11 có sẵn)

```powershell
powershell -ExecutionPolicy Bypass -File scripts\windows\build.ps1
```

Kết quả nằm trong `apps\desktop\src-tauri\target\release\bundle\` (NSIS installer + MSI).

## 4. Cập nhật

Mở **Cài đặt → Cập nhật** trong app:

- **Kiểm tra**: gọi GitHub Releases, so sánh phiên bản.
- **Tải bản phù hợp**: tải asset Windows vào `%APPDATA%\tez\updates` (thư mục dữ liệu Tauri).
- **Mở file đã tải**: chạy installer.

Hoặc dùng CLI:

```powershell
tez update
tez update --download
```

Dữ liệu trong `%APPDATA%\dev.teaching-ez.tez` (hoặc `%APPDATA%\tez` tùy bản cài) không bị xóa khi cập nhật.

## 5. Gỡ cài đặt

- Bản MSI/NSIS: **Settings → Apps → tez → Uninstall**.
- Bản zip: xóa `%LOCALAPPDATA%\tez`, shortcut Start Menu và dòng PATH nếu muốn.
- Dữ liệu: xóa thư mục dữ liệu trong **Cài đặt → Dữ liệu**.

## Ghi chú an toàn

Chế độ Code chỉ thao tác trong workspace bạn chọn. Đây là giới hạn đường dẫn ở tầng ứng dụng, không phải sandbox hệ điều hành. Không chạy tez bằng tài khoản Administrator cho công việc thường ngày.
