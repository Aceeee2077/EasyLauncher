import { useEffect, useState } from "react";
import { HardDrive, RefreshCw, Trash2 } from "lucide-react";
import { Button, Modal, bytes } from "./ui";
import { call } from "../services/api";
import { useI18n, type MessageKey } from "../i18n";
import type { DiskReport } from "../types";
import type { TaskProps } from "../utils/task";

/** Per-server folder breakdown with a checked cleanup of regenerable files. */
export function DiskUsage({ id, run, busy }: TaskProps & { id: string }) {
  const { t } = useI18n();
  const [report, setReport] = useState<DiskReport | null>(null);
  const [picked, setPicked] = useState<Set<string>>(new Set());
  const [confirm, setConfirm] = useState(false);
  const [message, setMessage] = useState("");
  const key = `disk-${id}`;
  const load = async () => {
    setReport(await call("disk_report", { id }));
  };
  useEffect(() => {
    void run(`disk-load-${id}-${crypto.randomUUID()}`, load);
  }, [id]);
  const toggle = (target: string) =>
    setPicked((prev) => {
      const next = new Set(prev);
      if (!next.delete(target)) next.add(target);
      return next;
    });
  const selected = report?.cleanups.filter((item) => picked.has(item.id)) ?? [];
  const freeing = selected.reduce((sum, item) => sum + item.bytes, 0);
  const clean = () => {
    void run(`disk-clean-${id}`, async () => {
      const result = await call("clean_disk", {
        id,
        targets: selected.map((item) => item.id),
        confirmed: true,
      });
      setConfirm(false);
      setPicked(new Set());
      setMessage(
        t("disk.cleaned", {
          size: bytes(result.freedBytes),
          count: result.removedFiles,
        }) + (result.failures.length ? ` ${t("disk.failed", { count: result.failures.length })}` : ""),
      );
      await load();
    });
  };
  const total = report?.totalBytes ?? 0;
  return (
    <section className="panel">
      <div className="section-title">
        <div>
          <h2>
            <HardDrive size={18} />
            {t("disk.title")}
          </h2>
          <p>{t("disk.desc")}</p>
        </div>
        <Button
          busy={busy.has(key)}
          disabled={!report}
          onClick={() => void run(key, load)}
        >
          <RefreshCw size={15} />
          {t("ops.refresh")}
        </Button>
      </div>
      {report ? (
        <>
          <div className="disk-summary">
            <div>
              <small>{t("disk.used")}</small>
              <strong>{bytes(report.totalBytes)}</strong>
            </div>
            <div>
              <small>{t("disk.free")}</small>
              <strong>
                {report.freeBytes === null ? "—" : bytes(report.freeBytes)}
              </strong>
            </div>
            <p>{t("disk.summary")}</p>
          </div>
          <div className="disk-list">
            {report.categories.map((item) => {
              const share = total ? (item.bytes / total) * 100 : 0;
              return (
                <div className="disk-row" key={item.id}>
                  <span>
                    <strong>
                      {t(`disk.category.${item.id}` as MessageKey)}
                    </strong>
                    <small>
                      {t("disk.files", {
                        count: item.files,
                        size: bytes(item.bytes),
                      })}
                    </small>
                  </span>
                  <div className="disk-track">
                    <i style={{ width: `${share.toFixed(1)}%` }} />
                  </div>
                  <b>{bytes(item.bytes)}</b>
                </div>
              );
            })}
          </div>
          <div className="disk-cleanups">
            <h3>{t("disk.cleanupTitle")}</h3>
            <p>{t("disk.cleanupDesc")}</p>
            {report.locked && (
              <p className="disk-locked">{t("disk.locked")}</p>
            )}
            {report.cleanups.length ? (
              report.cleanups.map((item) => (
                <label
                  className={`disk-cleanup${report.locked ? " disabled" : ""}`}
                  key={item.id}
                >
                  <input
                    type="checkbox"
                    checked={picked.has(item.id)}
                    disabled={report.locked}
                    onChange={() => toggle(item.id)}
                  />
                  <span>
                    <strong>
                      {t(`disk.cleanup.${item.id}` as MessageKey)}
                    </strong>
                    <small>
                      {t("disk.files", {
                        count: item.files,
                        size: bytes(item.bytes),
                      })}
                    </small>
                    {item.sample.length > 0 && (
                      <code>{item.sample.slice(0, 3).join(" · ")}</code>
                    )}
                  </span>
                  <b>{bytes(item.bytes)}</b>
                </label>
              ))
            ) : (
              <p className="mini-empty">{t("disk.nothing")}</p>
            )}
            {report.cleanups.length > 0 && (
              <div className="disk-actions">
                <Button
                  variant="danger"
                  disabled={!selected.length || report.locked}
                  onClick={() => setConfirm(true)}
                >
                  <Trash2 size={14} />
                  {t("disk.clean", { size: bytes(freeing) })}
                </Button>
                {message && (
                  <span className="success-text small" role="status">
                    {message}
                  </span>
                )}
              </div>
            )}
          </div>
        </>
      ) : (
        <p className="mini-empty">{t("common.loading")}</p>
      )}
      {confirm && (
        <Modal title={t("disk.confirmTitle")} onClose={() => setConfirm(false)}>
          <p>{t("disk.confirmBody", { size: bytes(freeing) })}</p>
          <div className="modal-actions">
            <Button onClick={() => setConfirm(false)}>
              {t("common.cancel")}
            </Button>
            <Button variant="danger" busy={busy.has(`disk-clean-${id}`)} onClick={clean}>
              {t("disk.confirm")}
            </Button>
          </div>
        </Modal>
      )}
    </section>
  );
}
