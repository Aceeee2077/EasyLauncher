import { useEffect, useState } from "react";
import {
  Check,
  Copy,
  FileCode2,
  RefreshCw,
  ServerCog,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { Button, Modal } from "./ui";
import { call } from "../services/api";
import { useI18n, type MessageKey } from "../i18n";
import type { ServicePreview, SupervisorSupport } from "../types";
import type { TaskProps } from "../utils/task";

/** Reviews and writes a systemd unit or launchd plist for a server. */
export function ProductionService({ id, run, busy }: TaskProps & { id: string }) {
  const { t } = useI18n();
  const [support, setSupport] = useState<SupervisorSupport | null>(null);
  const [preview, setPreview] = useState<ServicePreview | null>(null);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">(
    "idle",
  );
  const [confirm, setConfirm] = useState<"install" | "remove" | null>(null);
  const [message, setMessage] = useState("");
  const key = `service-${id}`;
  const load = async () => {
    const next = await call("supervision_info");
    setSupport(next);
    setMessage("");
    setCopyState("idle");
    if (!next.available) {
      setPreview(null);
      return;
    }
    setPreview(await call("supervision_preview", { id }));
  };
  useEffect(() => {
    void run(`service-load-${id}-${crypto.randomUUID()}`, load);
  }, [id]);
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(preview?.contents ?? "");
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  };
  const apply = (mode: "install" | "remove") => {
    void run(key, async () => {
      const next =
        mode === "install"
          ? await call("supervision_install", { id, confirmed: true })
          : await call("supervision_remove", { id, confirmed: true });
      setPreview(next);
      setConfirm(null);
      setMessage(
        t(mode === "install" ? "service.installDone" : "service.removeDone"),
      );
    });
  };
  const reason = support?.reason ?? "unsupportedOs";
  return (
    <section className="panel">
      <div className="section-title">
        <div>
          <h2>
            <ServerCog size={18} />
            {t("service.title")}
          </h2>
          <p>{t("service.desc")}</p>
        </div>
        <Button
          busy={busy.has(key)}
          disabled={!support}
          onClick={() => void run(key, load)}
        >
          <RefreshCw size={15} />
          {t("service.refresh")}
        </Button>
      </div>
      {!support ? (
        <p className="mini-empty">{t("common.loading")}</p>
      ) : !support.available ? (
        <div className="mini-empty">
          <TriangleAlert size={36} />
          <h3>{t("service.unsupportedTitle")}</h3>
          <p>{t(`service.reason.${reason}` as MessageKey)}</p>
        </div>
      ) : preview ? (
        <>
          <div className="service-meta">
            <span className="tag info">
              {t(`service.kind.${preview.kind}` as MessageKey)}
            </span>
            <span className={`tag ${preview.installed ? "ok" : "info"}`}>
              {t(
                preview.installed
                  ? "service.installed"
                  : "service.notInstalled",
              )}
            </span>
          </div>
          {preview.active && (
            <p className="service-warning">
              <TriangleAlert size={16} />
              {t("service.activeWarning")}
            </p>
          )}
          <p className="service-warning">
            <TriangleAlert size={16} />
            {t("service.invisibleWarning")}
          </p>
          {!preview.absoluteJava && (
            <p className="service-warning">
              <TriangleAlert size={16} />
              {t("service.javaWarning", { java: preview.javaPath })}
            </p>
          )}
          <div className="service-paths">
            <p>
              <small>{t("service.file")}</small>
              <code>{preview.path}</code>
            </p>
            <p>
              <small>{t("service.logFile")}</small>
              <code>{preview.logPath}</code>
            </p>
          </div>
          <p className="service-note">
            {t(
              preview.kind === "systemd"
                ? "service.systemdNote"
                : "service.launchdNote",
              { memory: `${preview.memoryMb} MB` },
            )}
          </p>
          <div className="service-block">
            <div className="service-block-head">
              <h3>
                <FileCode2 size={15} />
                {t("service.contents")}
              </h3>
              <Button variant="ghost" onClick={() => void copy()}>
                {copyState === "copied" ? (
                  <Check size={15} />
                ) : (
                  <Copy size={15} />
                )}
                {copyState === "copied"
                  ? t("service.copied")
                  : t("service.copy")}
              </Button>
            </div>
            <pre className="service-code">{preview.contents}</pre>
            {copyState === "failed" && (
              <p className="diag-copy-failed">{t("service.copyFailed")}</p>
            )}
          </div>
          <div className="service-block">
            <div className="service-block-head">
              <h3>{t("service.commands")}</h3>
            </div>
            <div className="service-commands">
              {preview.commands.map((command) => (
                <code key={command}>{command}</code>
              ))}
            </div>
            <p className="service-note">{t("service.commandsHint")}</p>
          </div>
          <div className="service-actions">
            <Button onClick={() => setConfirm("install")}>
              <FileCode2 size={14} />
              {t(
                preview.installed ? "service.update" : "service.install",
              )}
            </Button>
            {preview.installed && (
              <Button variant="danger" onClick={() => setConfirm("remove")}>
                <Trash2 size={14} />
                {t("service.remove")}
              </Button>
            )}
            {message && (
              <span className="success-text small" role="status">
                {message}
              </span>
            )}
          </div>
        </>
      ) : (
        <p className="mini-empty">{t("common.loading")}</p>
      )}
      {confirm && preview && (
        <Modal
          title={t(
            confirm === "install"
              ? "service.confirmInstallTitle"
              : "service.confirmRemoveTitle",
          )}
          onClose={() => setConfirm(null)}
        >
          <p>
            {t(
              confirm === "install"
                ? "service.confirmInstallBody"
                : "service.confirmRemoveBody",
              { path: preview.path, log: preview.logPath },
            )}
          </p>
          <div className="modal-actions">
            <Button onClick={() => setConfirm(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant={confirm === "remove" ? "danger" : "primary"}
              busy={busy.has(key)}
              onClick={() => apply(confirm)}
            >
              {t(
                confirm === "install"
                  ? "service.confirmInstall"
                  : "service.confirmRemove",
              )}
            </Button>
          </div>
        </Modal>
      )}
    </section>
  );
}
