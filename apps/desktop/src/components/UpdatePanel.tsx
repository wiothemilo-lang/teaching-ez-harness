import { useCallback, useEffect, useState } from "react";
import * as api from "../api";
import type { DownloadUpdateResult, ToastKind, UpdateInfo } from "../types";
import { IconDownload, IconRefresh, IconSparkle } from "../icons";
import { Callout, Spinner, Tag } from "./ui";

type Props = {
  current: string;
  notify: (kind: ToastKind, title: string, detail?: string) => void;
};

export function UpdatePanel({ current, notify }: Props) {
  const [info, setInfo] = useState<UpdateInfo | null>(null);
  const [checking, setChecking] = useState(false);
  const [downloading, setDownloading] = useState(false);
  const [downloaded, setDownloaded] = useState<DownloadUpdateResult | null>(null);
  const [error, setError] = useState<string | null>(null);

  const check = useCallback(
    async (silent = false) => {
      setChecking(true);
      setError(null);
      try {
        const next = await api.checkUpdate();
        setInfo(next);
        if (!silent) {
          notify(
            next.available ? "info" : "success",
            next.available ? `Có bản mới ${next.latest}` : "Bạn đang dùng bản mới nhất",
            next.available ? "Xem ghi chú và tải bản cập nhật bên dưới." : `Phiên bản ${next.current}`,
          );
        }
      } catch (checkError) {
        const message = api.errorMessage(checkError);
        setError(message);
        if (!silent) notify("error", "Không kiểm tra được cập nhật", message);
      } finally {
        setChecking(false);
      }
    },
    [notify],
  );

  useEffect(() => {
    void check(true);
  }, [check]);

  const download = async () => {
    setDownloading(true);
    try {
      const result = await api.downloadUpdate();
      setDownloaded(result);
      notify("success", `Đã tải tez ${result.version}`, result.path);
    } catch (downloadError) {
      notify("error", "Không tải được bản cập nhật", api.errorMessage(downloadError));
    } finally {
      setDownloading(false);
    }
  };

  const launch = async () => {
    if (!downloaded) return;
    try {
      await api.launchUpdate(downloaded.path);
    } catch (launchError) {
      notify("error", "Không mở được bản cập nhật", api.errorMessage(launchError));
    }
  };

  const openRelease = async () => {
    if (!info) return;
    try {
      await api.openExternal(info.url);
    } catch (openError) {
      notify("error", "Không mở được trang phát hành", api.errorMessage(openError));
    }
  };

  return (
    <section className="settings-section">
      <div className="section-head">
        <div>
          <h3>Cập nhật</h3>
          <p>
            tez kiểm tra GitHub Releases khi bạn mở tab này. Không có telemetry, không tự động tải
            ngầm.
          </p>
        </div>
        <button className="btn" onClick={() => void check()} disabled={checking}>
          {checking ? <Spinner label="Đang kiểm tra…" /> : <><IconRefresh size={15} /> Kiểm tra</>}
        </button>
      </div>

      <div className="update-panel">
        <div className="update-hero">
          <div className="update-mark" aria-hidden="true">
            <IconSparkle size={22} />
          </div>
          <div className="update-copy">
            <div className="update-version">
              <span>Phiên bản hiện tại</span>
              <code>v{info?.current ?? current}</code>
            </div>
            {checking ? (
              <p className="update-status">Đang hỏi GitHub Releases…</p>
            ) : error ? (
              <p className="update-status is-error">{error}</p>
            ) : info ? (
              <p className={`update-status ${info.available ? "is-new" : "is-current"}`}>
                {info.available
                  ? `Có bản mới v${info.latest}`
                  : "Bạn đang dùng bản mới nhất."}
              </p>
            ) : (
              <p className="update-status">Chưa kiểm tra.</p>
            )}
          </div>
          {info?.available && <Tag tone="forest">v{info.latest}</Tag>}
        </div>

        {info?.available && (
          <>
            {info.notes.trim() && (
              <div className="update-notes">
                <h4>Ghi chú phát hành</h4>
                <pre>{info.notes.trim()}</pre>
              </div>
            )}

            {info.assets.length > 0 && (
              <div className="update-assets">
                <h4>File phát hành</h4>
                {info.assets.map((asset) => (
                  <div className="update-asset" key={asset.name}>
                    <code>{asset.name}</code>
                    <span>{formatSize(asset.size)}</span>
                  </div>
                ))}
              </div>
            )}

            <div className="update-actions">
              <button
                className="btn btn-primary"
                onClick={() => void download()}
                disabled={downloading}
              >
                <IconDownload size={15} />
                {downloading ? "Đang tải…" : "Tải bản phù hợp"}
              </button>
              <button className="btn" onClick={() => void openRelease()}>
                Mở trang phát hành
              </button>
              {downloaded && (
                <button className="btn" onClick={() => void launch()}>
                  Mở file đã tải
                </button>
              )}
            </div>

            {downloaded && (
              <Callout>
                Đã tải về <code>{downloaded.path}</code>. Trên Windows, chạy file cài đặt và làm
                theo hướng dẫn. Trên Linux, cài gói .deb/.rpm hoặc thay AppImage hiện tại.
              </Callout>
            )}
          </>
        )}

        <Callout>
          <strong>Cập nhật thủ công.</strong> Bạn luôn có thể tải bản mới nhất tại{" "}
          <code>github.com/teaching-ez/tez/releases</code> và cài đè lên bản cũ. Dữ liệu trong thư
          mục người dùng không bị xóa.
        </Callout>
      </div>
    </section>
  );
}

function formatSize(bytes: number): string {
  if (!bytes) return "";
  const mb = bytes / (1024 * 1024);
  if (mb >= 1) return `${mb.toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1024))} KB`;
}
