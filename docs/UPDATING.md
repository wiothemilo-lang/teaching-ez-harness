# Kiểm tra và cập nhật

tez không dùng updater tự động chạy ngầm. Việc kiểm tra do người dùng chủ động, và mọi request chỉ tới GitHub API.

## Trong desktop app

Mở **Cài đặt → Cập nhật**:

1. App gọi `https://api.github.com/repos/<repo>/releases/latest`.
2. So sánh `tag_name` với phiên bản đang chạy (semver đơn giản).
3. Hiển thị ghi chú phát hành và danh sách asset.
4. **Tải bản phù hợp** chọn asset theo hệ điều hành và lưu vào thư mục dữ liệu/`updates`.
5. **Mở file đã tải** để chạy installer (Windows) hoặc mở thư mục (Linux).

Không có token, không telemetry. Nếu repo riêng tư, đặt biến môi trường `TEZ_GITHUB_REPO=owner/name`; API công khai không hỗ trợ repo private nếu không có token.

## CLI

```bash
tez update             # kiểm tra
tez update --download  # tải asset phù hợp
```

## Phát hành bản mới

1. Tăng version trong:
   - `Cargo.toml` (`workspace.package.version`)
   - `apps/desktop/package.json`
   - `apps/desktop/src-tauri/Cargo.toml`
   - `apps/desktop/src-tauri/tauri.conf.json`
2. Commit và tạo tag:

```bash
git tag v0.2.0
git push origin v0.2.0
```

3. Workflow `.github/workflows/release.yml` build Linux + Windows, tạo GitHub Release và đính kèm installer cùng CLI.

## Ghi chú kỹ thuật

- So sánh phiên bản nằm trong `crates/tez-update/src/lib.rs` (`is_newer`).
- Asset được chọn theo tên file: `windows|win64|x64|.msi|.exe`, `linux|appimage|.deb|.rpm`, `macos|dmg`.
- Tải về không tự chạy. Người dùng quyết định cài.
- Nếu GitHub rate-limit (60 request/giờ cho IP chưa xác thực), thử lại sau hoặc mở trang Releases thủ công.
