import { useEffect } from "react";
import type { ApprovalRequest } from "../types";
import { IconLock, IconX } from "../icons";

type Props = {
  request: ApprovalRequest;
  onRespond: (approved: boolean) => void;
};

/** Hộp thoại chặn luồng chat khi agent xin quyền chạy một tool rủi ro. */
export function ApprovalDialog({ request, onRespond }: Props) {
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") onRespond(false);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [onRespond]);

  const args = JSON.stringify(request.args ?? {}, null, 2);
  const preview = request.preview?.trim() ? request.preview : null;

  return (
    <div className="approval-backdrop" role="dialog" aria-modal="true" aria-labelledby="approval-title">
      <div className="approval-card">
        <div className="approval-head">
          <span className="approval-icon">
            <IconLock size={17} />
          </span>
          <div className="approval-copy">
            <p className="approval-kicker">CẦN DUYỆT</p>
            <h2 id="approval-title">{request.reason}</h2>
          </div>
          <button
            type="button"
            className="approval-close"
            aria-label="Từ chối và đóng"
            onClick={() => onRespond(false)}
          >
            <IconX size={16} />
          </button>
        </div>

        <div className="approval-tool">
          <span>Tool</span>
          <code>{request.tool}</code>
        </div>
        {preview ? (
          <>
            <div className="approval-tool">
              <span>Thay đổi</span>
              <code>diff trước khi ghi</code>
            </div>
            <pre className="approval-diff">
              {preview.split("\n").map((line, index) => (
                <span
                  key={index}
                  className={
                    line.startsWith("+") && !line.startsWith("+++")
                      ? "diff-add"
                      : line.startsWith("-") && !line.startsWith("---")
                        ? "diff-del"
                        : line.startsWith("@@")
                          ? "diff-hunk"
                          : "diff-line"
                  }
                >
                  {line}
                  {"\n"}
                </span>
              ))}
            </pre>
          </>
        ) : (
          <pre className="approval-args">{args}</pre>
        )}
        <p className="approval-note">
          Chỉ cho phép khi bạn hiểu thao tác này. Từ chối sẽ dừng tool và agent sẽ nhận được
          thông báo để thử cách khác.
        </p>

        <div className="approval-actions">
          <button type="button" className="btn" onClick={() => onRespond(false)}>
            Từ chối
          </button>
          <button type="button" className="btn btn-primary" onClick={() => onRespond(true)}>
            Cho phép chạy
          </button>
        </div>
      </div>
    </div>
  );
}
