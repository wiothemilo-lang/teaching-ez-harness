# Eval hồi quy

Bộ eval tối giản để phát hiện regression rõ ràng của provider/model sau khi đổi
prompt, tool hoặc cấu hình. Đây không phải bài kiểm tra chất lượng sư phạm đầy
đủ; nó chỉ kiểm tra câu trả lời có chứa dữ kiện bắt buộc và không chứa đáp án
sai đã biết.

## Chạy

```bash
tez eval
tez eval --suite evals/basic.json
tez eval --suite evals/basic.json --json
tez eval --suite evals/basic.json --min-pass 8
```

Mặc định dùng provider đang active. Có thể ghi đè:

```bash
tez eval --provider groq --model llama-3.3-70b-versatile
```

Lệnh trả mã thoát khác 0 nếu có case fail (hoặc không đạt `--min-pass`), nên
dùng được trong CI.

## Định dạng

```json
{
  "name": "basic",
  "cases": [
    {
      "id": "math-arithmetic",
      "subject": "math",
      "prompt": "Tính 17 x 23. Chỉ trả lời một số.",
      "system": "Trả lời ngắn gọn.",
      "contains": ["391"],
      "not_contains": ["392"],
      "max_tokens": 64,
      "temperature": 0
    }
  ]
}
```

- `contains`: tất cả chuỗi phải xuất hiện, không phân biệt hoa thường.
- `not_contains`: không được xuất hiện chuỗi nào.
- Mỗi case phải có ít nhất một tiêu chí `contains`/`not_contains`.
- `subject`, `system`, `max_tokens`, `temperature` là tùy chọn.

## Lưu ý

- Kết quả phụ thuộc model; case quá chặt sẽ fail vì khác định dạng, không phải
  vì model sai. Ưu tiên dữ kiện ngắn, không phụ thuộc văn phong.
- Không đưa API key vào file eval. Dùng `tez connect` hoặc biến môi trường.
- Bộ eval với provider thật nên chạy nightly hoặc trước release, không chạy
  trong mọi lần CI công khai để tránh lộ key và tốn chi phí.
