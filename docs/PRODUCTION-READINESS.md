# Đánh giá mức sẵn sàng production

**Cập nhật:** 2026-11-15
**Phạm vi:** workspace `teaching-ez-harness` (Rust workspace + desktop Tauri).
**Kết luận ngắn:** bản hiện tại đủ cho **beta nội bộ / nhóm nhỏ tin cậy**. Đã có backup/restore, log bền vững, sandbox `run_command` trên Linux, sandbox MCP stdio tùy chọn và eval hồi quy; chưa nên phát hành rộng rãi cho đến khi xử lý các mục P0 còn lại: sandbox MCP mặc định/non-Linux, kiểm soát egress toàn diện và ký số installer.

## 1. Đã sửa trong đợt này

| ID | Mức | Vấn đề | Thay đổi | Kiểm chứng |
|---|---|---|---|---|
| CI-01 | P0 | Không có CI; release workflow chỉ build khi tag, không chặn fmt/clippy/test | Thêm `.github/workflows/ci.yml`: fmt, clippy `-D warnings`, test workspace; job desktop chạy `tsc`, `vite build`, `cargo check` Tauri | Workflow mới; chạy local tương đương bên dưới |
| CMD-01 | P0 | Huỷ agent không dừng process cháu của `run_command` | Thêm `ProcessTreeGuard` trong `crates/tez-core/src/tools/workspace.rs`; guard kill cả process group khi future bị drop, disarm ngay sau khi child được reap để tránh PID reuse | Test `cancelling_command_kills_child_processes` |
| MCP-01 | P0 | `mcp.json` toàn cục hỏng làm hỏng cả lượt chat | `load_servers_with_project` dùng loader lenient; file hỏng bị bỏ qua kèm cảnh báo, Settings vẫn báo lỗi rõ | Test `malformed_global_config_is_skipped` |
| MCP-02 | P0 | Ghi `mcp.json` không atomic, không giữ bản backup khi file hỏng | Ghi file tạm cùng thư mục + `sync_all` + rename; file hỏng được đổi tên `mcp.json.corrupt-<timestamp>-<pid>` trước khi ghi đè | Test `save_config_backs_up_corrupt_file` |
| MCP-03 | P0 | MCP stdio kế thừa toàn bộ env, làm lộ API key/token của app | `inherit_safe_env`: `env_clear` rồi chỉ chuyển `PATH`, `HOME`, `LANG`, ...; biến bổ sung phải khai báo trong `env` của server | `cargo test -p tez-core`; kiểm tra thủ công với server cần env |
| LLM-01 | P0 | Lỗi 429/5xx hoặc timeout làm hỏng lượt ngay lập tức | Retry tối đa 2 lần với backoff 500 ms → 8 s; tôn trọng `Retry-After` (tối đa 10 s); chỉ retry 429/5xx và lỗi connect/timeout | Test `retries_transient_api_errors`, `retries_transient_stream_errors_before_first_byte` |
| AUD-01 | P1 | `audit.jsonl` tăng vô hạn | Xoay sang `audit.jsonl.1` khi vượt 5 MB; giữ quyền 0600 | Test `audit_log_rotates_when_full` |
| DB-01 | P1 | `tez doctor` không kiểm tra toàn vẹn DB | Thêm `Store::quick_check` và in `db integrity: ok/LỖI` trong `tez doctor` | `cargo test -p tez-store`; chạy `tez doctor` |
| UI-01 | P1 | Desktop không typecheck được vì thiếu `McpExtrasDialog` | Bổ sung dialog resources/prompts dùng `mcp_extras` | `pnpm exec tsc --noEmit`, `pnpm build` |
| DATA-01 | P0 | Chưa có lệnh backup/restore; `doctor` mới chỉ `quick_check` | Thêm crate `tez-backup`: snapshot SQLite bằng backup API, manifest kích thước + SHA-256, kiểm tra `quick_check`, chặn path traversal, tự tạo bản an toàn trước khi phục hồi; CLI `tez backup`/`tez restore`; desktop Settings → Dữ liệu | 4 test `tez-backup`; roundtrip CLI thủ công |
| LOG-01 | P1 | Desktop chỉ dùng `eprintln`, không có log bền vững/panic hook khi mở từ GUI | Logger `logs/tez.log` xoay 5 MB + panic hook ghi backtrace; CLI `tez logs`; desktop khởi tạo logger và ghi lỗi qua `friendly` | 2 test `tez-core::logging`; chạy `tez logs` |
| SEC-02 | P0 | `run_command` đọc được `master.key`/`providers.json` và gửi dữ liệu ra ngoài | Sandbox bubblewrap trên Linux: workspace read-write, data dir/credential trong HOME bị ẩn, network namespace tách mặc định; env con chỉ còn allowlist; fail-closed nếu thiếu `bwrap` | Test `sandbox_hides_app_data_dir`; `tez doctor` hiển thị trạng thái sandbox |
| SEC-03 | P0 | MCP stdio chạy với quyền user, đọc được secret trong data dir và gửi dữ liệu ra ngoài | Sandbox bubblewrap tùy chọn theo server: ẩn `master.key`/`providers.json`/DB/audit, chặn mạng mặc định, chỉ bind `writable_paths`; thiếu `bwrap` thì từ chối; UI Settings → MCP | Test `sandboxed_demo_server_discovers_tools`, `mcp_command_hides_secrets_and_blocks_network` |
| EVAL-01 | P1 | Chưa có eval harness/regression | Thêm `tez eval --suite evals/*.json`: case JSON, kiểm tra `contains`/`not_contains`, báo cáo text/JSON, mã thoát khác 0 khi fail; 8 case mẫu; workflow nightly tự bỏ qua nếu thiếu secret | 4 test `tez-cli`; chạy mock 8/8 pass; `tez eval --json` |
| CTX-02 | P1 | Provider trả `400 Invalid request` chung chung khi context lớn; compaction cũ chỉ chạy khi vượt giới hạn cứng | Hạ mặc định còn 24.000 ký tự; micro-compact ở 80% và nén về 65%; summary ở 90%; nhận diện `400 Invalid request`/`413` khi context đã dùng ≥50% và thử summary một lần | Test `detects_generic_invalid_request`, `retries_generic_invalid_request_when_context_is_tight`; 395 test workspace |
| MCP-05 | P2 | HTTP chỉ nhận response qua POST, bỏ qua request/notification server khởi tạo | Mở GET SSE best-effort sau `notifications/initialized`; trả lời `ping`/`roots/list`, request khác trả `-32601`; task bị abort khi connection drop | Test `http_get_stream_answers_server_ping` |

### Lệnh kiểm chứng

```bash
PATH=/root/.cargo/bin:$PATH cargo fmt --all -- --check
PATH=/root/.cargo/bin:$PATH cargo clippy --workspace --all-targets -- -D warnings
PATH=/root/.cargo/bin:$PATH cargo test --workspace
cd apps/desktop && pnpm exec tsc --noEmit && pnpm build
PATH=/root/.cargo/bin:$PATH cargo check --manifest-path apps/desktop/src-tauri/Cargo.toml
```

Kết quả local ngày 2026-11-15: fmt sạch, clippy sạch, **395 test pass**, desktop typecheck/build pass, Tauri check pass.

## 2. Còn lại trước khi phát hành rộng rãi

| ID | Mức | Vấn đề | Giảm thiểu hiện tại | Hướng xử lý |
|---|---|---|---|---|
| SEC-01 | **P0** | MCP stdio chưa sandbox mặc định và `run_command` trên macOS/Windows chưa có sandbox OS; prompt injection có thể đọc secret hoặc gửi dữ liệu ra ngoài | `run_command` đã sandbox trên Linux; MCP stdio có sandbox bubblewrap tùy chọn theo server; env allowlist; permission mode/rule, deny-list | Bật sandbox MCP mặc định/khuyến nghị, sandbox macOS/Windows, network allowlist toàn diện |
| KEY-01 | P1 | Vault dùng `master.key` cạnh dữ liệu, chưa dùng OS keychain; bản backup chứa cả `master.key` | File 0600; cảnh báo trong README | Keychain/Secret Service/DPAPI, hoặc passphrase mở khoá; mã hoá bản backup |
| SIGN-01 | P1 | Installer Windows chưa ký; update chưa tự cài | Release thủ công, checksum | Ký Authenticode + auto-update có xác minh chữ ký |
| EVAL-02 | P1 | Chưa chạy eval với provider thật và chưa lưu baseline | Có `tez eval` + 8 case mock; workflow nightly bỏ qua nếu thiếu secret | Cấu hình secret, chạy nightly, lưu baseline và ngưỡng pass |
| CTX-01 | P2 | Ước lượng token theo ký tự; chưa có context inspector | Compaction 4 bước, dock hiển thị context | Tokenizer theo model + `context inspect` đọc `context_snapshots` |
| MCP-04 | P2 | MCP stdio spawn mỗi lượt, timeout 20 s; GET stream server-initiated mới ở mức best-effort, không reconnect | Timeout + kill_on_drop; GET SSE trả lời `ping`/`roots/list`, request khác trả `-32601` | Giữ process dài hạn, backoff/reconnect cho GET stream |

## 3. Tiêu chí go-live

- [x] CI chặn fmt/clippy/test và build desktop.
- [x] Huỷ thao tác dừng cả cây process.
- [x] Cấu hình MCP hỏng không chặn chat; ghi cấu hình atomic.
- [x] Retry lỗi tạm thời của provider.
- [x] Audit log có rotation.
- [x] `doctor` kiểm tra toàn vẹn DB.
- [x] Backup/restore có manifest + SHA-256 + `quick_check`, có bản an toàn trước khi ghi đè.
- [x] Log bền vững + panic hook cho desktop/CLI.
- [x] Sandbox `run_command` trên Linux (bubblewrap, ẩn data dir/credential, chặn mạng mặc định).
- [x] Sandbox MCP stdio tùy chọn trên Linux (bubblewrap, ẩn secret, chặn mạng mặc định, `writable_paths`).
- [x] Compaction chạy sớm (24.000 ký tự, micro 80%, summary 90%) và phục hồi `400 Invalid request`/`413` khi context chịu áp lực.
- [x] Eval harness `tez eval` + 8 case mẫu + workflow nightly (tự bỏ qua nếu thiếu secret).
- [ ] Sandbox MCP mặc định/non-Linux + `run_command` trên macOS/Windows + network allowlist toàn diện.
- [ ] Ký số installer và auto-update xác minh chữ ký.
- [ ] Chạy eval với provider thật, lưu baseline và ngưỡng pass.

Chỉ nên coi là **production-ready** khi các mục chưa tick được xử lý và có bằng chứng chạy thật, không chỉ test mock.
