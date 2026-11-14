# Chính sách dung lượng & dọn dẹp

> Mục tiêu: mọi thứ "tự lớn" (toolchain, cache, build artifacts, log, DB, model local) đều có **hạn mức** và **quy trình dọn dẹp**. Không bao giờ tự xóa source code, tài liệu bài học, hay dữ liệu người dùng khi chưa được xác nhận.

---

## 1. Ngân sách dung lượng

Máy hiện có ~131GB trống. Ngân sách cho dự án:

| Hạng mục | Đường dẫn | Mềm | Cứng | Khi vượt |
|---|---|---:|---:|---|
| Rust toolchain | `~/.rustup` | 2GB | 3GB | Bỏ toolchain cũ, chỉ giữ stable |
| Cargo registry | `~/.cargo/registry` | 3GB | 4GB | Xóa cache > 30 ngày |
| Cargo git | `~/.cargo/git` | 1GB | 2GB | Xóa checkout > 30 ngày |
| Build artifacts | `target/` | 8GB | 12GB | Xóa `incremental`, rồi `cargo clean` |
| Desktop build | `apps/desktop/src-tauri/target/` | 7GB | 9GB | Xóa `incremental`, rồi `cargo clean` |
| Node runtime | `~/.local/share/fnm` | 1GB | 2GB | Chỉ giữ một bản LTS |
| pnpm store | `~/.local/share/pnpm/store` | 3GB | 5GB | `pnpm store prune` |
| node_modules | `apps/*/node_modules` | 2GB | 3GB | Xóa cache/build tạm; cài lại khi cần |
| App data | `~/.local/share/teaching-ez` | 5GB | 8GB | Retention + VACUUM + xóa file mồ côi |
| Logs | `~/.local/share/teaching-ez/logs` | 200MB | 500MB | Rotate, giữ 7 ngày |
| Ollama models | `~/.ollama/models` | 15GB | 20GB | Chỉ xóa model không dùng, có xác nhận |
| **Tổng dự án** | — | **30GB** | **40GB** | Chạy dọn dẹp an toàn |

**Nguyên tắc:**

- Mềm = bắt đầu cảnh báo và dọn dẹp cơ hội.
- Cứng = bắt buộc dọn trước khi build/install tiếp.
- Tổng cứng 40GB, còn xa mức 131GB hiện có.

---

## 2. Cái gì được tự xóa, cái gì không

### Được tự xóa (an toàn)

- `target/`, `target/debug/incremental`, build cache.
- `~/.cargo/registry/cache`, `~/.cargo/git/checkouts` cũ.
- pnpm store không dùng; `node_modules/.cache`, `.vite`.
- Log > 7 ngày; temp files.
- Session > 90 ngày; tài liệu > 180 ngày **nếu người dùng bật retention**.
- SQLite `VACUUM` sau khi xóa.

### Không tự xóa

- Source code, `docs/`, `.git/`.
- Bài học, tài liệu gốc, lesson pack.
- API key/connection (trừ khi người dùng bấm xóa).
- Model Ollama đang dùng (chỉ xóa khi có xác nhận).
- Bất kỳ file nào ngoài các đường dẫn trong bảng.

---

## 3. Công cụ

### `scripts/disk-guard.sh`

```bash
scripts/disk-guard.sh check      # xem dung lượng, cảnh báo
scripts/disk-guard.sh clean      # dọn an toàn nếu vượt mềm
scripts/disk-guard.sh auto       # check + clean + check (dùng trước build)
```

- Cấu hình trong `scripts/disk-quotas.env`.
- Không cần quyền root.
- Không xóa dữ liệu người dùng nếu chưa bật retention.

### Trước khi build

```bash
scripts/disk-guard.sh auto
cargo build
pnpm build
```

Hoặc dùng wrapper (đã có):

```bash
scripts/build.sh   # disk guard + cargo build
scripts/test.sh    # disk guard + fmt + clippy + test
```

---

## 4. Quy định trong app (sau này)

`tez doctor` và Settings sẽ hiển thị:

- Dung lượng DB, tài liệu, log, cache.
- Hạn mức và mức sử dụng.
- Nút "Dọn dẹp ngay".
- Tùy chọn retention:
  - Session: 30/90/180 ngày.
  - Tài liệu: 90/180/365 ngày.
  - Log: 7/30 ngày.
  - Cache embedding: xóa khi không dùng.

**Mặc định đề xuất:**

| Dữ liệu | Retention |
|---|---|
| Log | 7 ngày |
| Session + trace | 90 ngày |
| Tài liệu ingest | 180 ngày |
| Context snapshot | 30 ngày |
| Lesson pack export | Không tự xóa |

**Khi DB > 5GB:**

1. Xóa `context_snapshots` > 30 ngày.
2. Xóa session > 90 ngày (kèm message/trace).
3. Xóa document > 180 ngày không dùng.
4. `VACUUM`.
5. Nếu vẫn > 8GB: cảnh báo người dùng chọn xóa thêm.

---

## 5. Log rotation

- Dùng `tracing-appender` rolling theo ngày.
- Giữ tối đa 7 file, mỗi file tối đa 50MB.
- Không log prompt/secret; chỉ metadata.
- Log level mặc định `info`; `debug` chỉ khi bật.

---

## 6. Ollama / model local

- Model mặc định gợi ý:
  - Chat: `qwen3:4b` (~2.5GB) hoặc `llama3.2:3b` (~2GB).
  - Embedding: `nomic-embed-text` (~274MB).
- Không pull model > 8GB nếu không cần.
- `tez doctor` liệt kê model + dung lượng.
- Xóa: `ollama rm <model>` (có xác nhận).
- Có thể đặt `OLLAMA_MODELS` sang ổ khác nếu cần.

---

## 7. Khi nào chạy dọn dẹp

- Trước mỗi lần build lớn.
- Mỗi tuần một lần (hoặc khi `disk-guard check` báo mềm).
- Sau khi package installer.
- Khi `tez doctor` báo > 80% hạn mức.

---

*Cập nhật khi có workspace Rust/Node thực tế.*
