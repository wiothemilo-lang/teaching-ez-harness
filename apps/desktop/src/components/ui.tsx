import type { ReactNode } from "react";
import { IconCheck, IconInfo, IconX } from "../icons";

export function Tag({
  children,
  tone = "neutral",
}: {
  children: ReactNode;
  tone?: "neutral" | "forest" | "clay" | "muted";
}) {
  return <span className={`tag tag-${tone}`}>{children}</span>;
}

export function Spinner({ label }: { label?: string }) {
  return (
    <span className="spinner-wrap" role="status">
      <span className="spinner" aria-hidden="true" />
      {label ? <span>{label}</span> : <span className="sr-only">Đang tải</span>}
    </span>
  );
}

export function Callout({
  children,
  tone = "forest",
}: {
  children: ReactNode;
  tone?: "forest" | "clay";
}) {
  return (
    <div className={`callout callout-${tone}`}>
      <IconInfo size={15} />
      <div>{children}</div>
    </div>
  );
}

export function EmptyNote({ children }: { children: ReactNode }) {
  return <p className="empty-note">{children}</p>;
}

export function OkMark({ ok }: { ok: boolean }) {
  return ok ? (
    <span className="ok-mark ok" title="Thành công">
      <IconCheck size={12} />
    </span>
  ) : (
    <span className="ok-mark fail" title="Lỗi">
      <IconX size={12} />
    </span>
  );
}
