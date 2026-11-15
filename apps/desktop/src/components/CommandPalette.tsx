import { useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { AppMode, SettingsTab } from "../types";
import {
  IconBook,
  IconChat,
  IconCode,
  IconPlan,
  IconDatabase,
  IconDownload,
  IconFolder,
  IconKey,
  IconPlug,
  IconPlus,
  IconPuzzle,
  IconSearch,
  IconSliders,
  IconSparkle,
} from "../icons";

type CreateKind = "skill" | "mcp" | "plugin";

type Props = {
  open: boolean;
  onClose: () => void;
  onNewSession: () => void;
  onMode: (mode: AppMode) => void;
  onOpenSettings: (tab: SettingsTab) => void;
  onOpenWorkspace: () => void;
  onCreate: (kind: CreateKind) => void;
};

type Action = {
  id: string;
  label: string;
  hint: string;
  icon: ReactNode;
  run: () => void;
};

export function CommandPalette({
  open,
  onClose,
  onNewSession,
  onMode,
  onOpenSettings,
  onOpenWorkspace,
  onCreate,
}: Props) {
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);

  const actions = useMemo<Action[]>(() => {
    const settings = (tab: SettingsTab, label: string, hint: string, icon: ReactNode): Action => ({
      id: `settings-${tab}`,
      label,
      hint,
      icon,
      run: () => onOpenSettings(tab),
    });

    return [
      {
        id: "new-session",
        label: "Cuộc trò chuyện mới",
        hint: "Ctrl N",
        icon: <IconPlus size={16} />,
        run: onNewSession,
      },
      {
        id: "mode-chat",
        label: "Chuyển sang Chat",
        hint: "Hỏi đáp, viết, lập kế hoạch",
        icon: <IconChat size={16} />,
        run: () => onMode("chat"),
      },
      {
        id: "mode-plan",
        label: "Chuyển sang Plan",
        hint: "Chỉ đọc, khảo sát và lập kế hoạch",
        icon: <IconPlan size={16} />,
        run: () => onMode("plan"),
      },
      {
        id: "mode-code",
        label: "Chuyển sang Code",
        hint: "Đọc, sửa file và chạy lệnh",
        icon: <IconCode size={16} />,
        run: () => onMode("code"),
      },
      {
        id: "workspace",
        label: "Chọn workspace",
        hint: "Thư mục agent được phép đọc/ghi",
        icon: <IconFolder size={16} />,
        run: onOpenWorkspace,
      },
      {
        id: "create-skill",
        label: "Tạo Skill bằng AI",
        hint: "Mô tả kỹ năng, AI viết SKILL.md",
        icon: <IconSparkle size={16} />,
        run: () => onCreate("skill"),
      },
      {
        id: "create-mcp",
        label: "Tạo MCP server bằng AI",
        hint: "Sinh cấu hình server stdio",
        icon: <IconPlug size={16} />,
        run: () => onCreate("mcp"),
      },
      {
        id: "create-plugin",
        label: "Tạo Plugin bằng AI",
        hint: "Gói skill + MCP thành plugin",
        icon: <IconPuzzle size={16} />,
        run: () => onCreate("plugin"),
      },
      settings("harness", "Cài đặt agent", "Quyền, model, context, memory", <IconSliders size={16} />),
      settings("providers", "Nhà cung cấp & model", "API key, endpoint, model", <IconKey size={16} />),
      settings("skills", "Quản lý kỹ năng", "Prompt pack Markdown", <IconBook size={16} />),
      settings("mcp", "Quản lý MCP server", "Công cụ ngoài qua stdio", <IconPlug size={16} />),
      settings("plugins", "Quản lý plugin", "Gói skill và MCP", <IconPuzzle size={16} />),
      settings("data", "Dữ liệu & phiên", "SQLite, thư mục, audit", <IconDatabase size={16} />),
      settings("update", "Cập nhật", "Kiểm tra bản mới", <IconDownload size={16} />),
      {
        id: "terminal",
        label: "Mở workspace trong file manager",
        hint: "Dùng thư mục hiện tại",
        icon: <IconFolder size={16} />,
        run: onOpenWorkspace,
      },
    ];
  }, [onCreate, onMode, onNewSession, onOpenSettings, onOpenWorkspace]);

  const filtered = useMemo(() => {
    const term = query.trim().toLowerCase();
    if (!term) return actions;
    return actions.filter(
      (action) =>
        action.label.toLowerCase().includes(term) || action.hint.toLowerCase().includes(term),
    );
  }, [actions, query]);

  useEffect(() => {
    if (!open) return;
    setQuery("");
    setActive(0);
    const timer = window.setTimeout(() => inputRef.current?.focus(), 20);
    return () => window.clearTimeout(timer);
  }, [open]);

  useEffect(() => {
    if (!open) return;
    const onKey = (event: globalThis.KeyboardEvent) => {
      if (event.key === "Escape") {
        event.preventDefault();
        onClose();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [open, onClose]);

  if (!open) return null;

  const run = (action: Action | undefined) => {
    if (!action) return;
    onClose();
    action.run();
  };

  return (
    <div className="command-overlay" role="presentation" onMouseDown={onClose}>
      <div
        className="command-box"
        role="dialog"
        aria-modal="true"
        aria-label="Bảng lệnh"
        onMouseDown={(event) => event.stopPropagation()}
      >
        <div className="command-input">
          <IconSearch size={17} />
          <input
            ref={inputRef}
            value={query}
            onChange={(event) => {
              setQuery(event.target.value);
              setActive(0);
            }}
            onKeyDown={(event) => {
              if (event.key === "ArrowDown") {
                event.preventDefault();
                setActive((current) => Math.min(current + 1, filtered.length - 1));
              } else if (event.key === "ArrowUp") {
                event.preventDefault();
                setActive((current) => Math.max(current - 1, 0));
              } else if (event.key === "Enter") {
                event.preventDefault();
                run(filtered[active]);
              }
            }}
            placeholder="Tìm lệnh, cài đặt hoặc hành động…"
            aria-label="Tìm lệnh"
          />
          <kbd>Esc</kbd>
        </div>

        <div className="command-list">
          {filtered.length === 0 ? (
            <p className="command-empty">Không có lệnh phù hợp.</p>
          ) : (
            filtered.map((action, index) => (
              <button
                key={action.id}
                className={`command-item ${index === active ? "is-active" : ""}`}
                onMouseEnter={() => setActive(index)}
                onClick={() => run(action)}
              >
                <span className="command-icon">{action.icon}</span>
                <span className="command-copy">
                  <strong>{action.label}</strong>
                  <small>{action.hint}</small>
                </span>
              </button>
            ))
          )}
        </div>
      </div>
    </div>
  );
}
