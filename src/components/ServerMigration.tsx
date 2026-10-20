import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Archive,
  Check,
  Download,
  FolderOpen,
  ShieldCheck,
  TriangleAlert,
} from "lucide-react";
import { Button, Field, Modal, bytes } from "./ui";
import { call, desktop } from "../services/api";
import { useI18n } from "../i18n";
import type {
  ExportPreview,
  ExportResult,
  MigrationProgress,
  ServerView,
} from "../types";
import type { TaskProps } from "../utils/task";
export function ServerMigration({
  server,
  run,
  busy,
}: TaskProps & { server: ServerView }) {
  const { t } = useI18n();
  const [info, setInfo] = useState<ExportPreview | null>(null);
  const [open, setOpen] = useState(false);
  const [path, setPath] = useState("");
  const [includeBackups, setIncludeBackups] = useState(false);
  const [progress, setProgress] = useState<MigrationProgress | null>(null);
  const [result, setResult] = useState<ExportResult | null>(null);
  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    const subscription = listen<MigrationProgress>(
      "migration-progress",
      (event) => {
        if (alive) setProgress(event.payload);
      },
    );
    return () => {
      alive = false;
      void subscription.then((f) => f());
    };
  }, []);
  const load = () =>
    run(`migration-info-${server.id}`, async () => {
      try {
        setInfo(await call("migration_export_preview", { id: server.id }));
      } catch {
        setInfo(null);
      }
    });
  useEffect(() => {
    if (desktop) {
      void run(`migration-info-${server.id}`, async () => {
        try {
          setInfo(await call("migration_export_preview", { id: server.id }));
        } catch {
          setInfo(null);
        }
      });
    }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [server.id]);
  const exporting = busy.has("migration-export");
  const openExport = () =>
    run(`migration-open-${server.id}`, async () => {
      setResult(null);
      setProgress(null);
      setIncludeBackups(false);
      setOpen(true);
      await load();
    });
  const browse = () =>
    run("migration-pick", async () => {
      const picked = await call("migration_pick_export", { name: server.name });
      if (picked) setPath(picked);
    });
  const submit = () =>
    run("migration-export", async () => {
      setProgress(null);
      const exported = await call("migration_export", {
        id: server.id,
        path: path.trim(),
        includeBackups,
      });
      setResult(exported);
      setInfo(null);
      void load();
    });
  const active =
    server.runtime.status === "running" ||
    server.runtime.status === "starting" ||
    server.runtime.status === "stopping" ||
    server.runtime.status === "restarting" ||
    server.runtime.status === "installing";
  const percent =
    progress && progress.totalBytes
      ? Math.min(
          100,
          Math.floor((progress.bytes / progress.totalBytes) * 100),
        )
      : 0;
  const canExport =
    !!path.trim() &&
    path.trim().toLowerCase().endsWith(".zip") &&
    !active &&
    !!info &&
    !info.running;
  return (
    <section className="panel">
      <div className="section-title">
        <div>
          <h2>
            <Archive size={18} />
            {t("migration.title")}
          </h2>
          <p>{t("migration.desc")}</p>
        </div>
        <Button disabled={!desktop} onClick={() => void openExport()}>
          <Download size={16} />
          {t("migration.export")}
        </Button>
      </div>
      <div className="migration-stats">
        <div>
          <small>{t("migration.included")}</small>
          <strong>
            {info ? bytes(info.sizeBytes) : t("common.unavailable")}
          </strong>
          <span>
            {info
              ? t("import.filesUnit", { count: info.fileCount })
              : t("migration.unknown")}
          </span>
        </div>
        <div>
          <small>{t("migration.backups")}</small>
          <strong>
            {info ? bytes(info.backupsBytes) : t("common.unavailable")}
          </strong>
          <span>
            {info
              ? t("import.filesUnit", { count: info.backupsFiles })
              : t("migration.unknown")}
          </span>
        </div>
        <div>
          <small>{t("migration.status")}</small>
          <strong>
            {active || info?.running
              ? t("migration.mustStop")
              : t("migration.ready")}
          </strong>
          <span>{server.port}</span>
        </div>
      </div>
      <ul className="import-notes">
        <li>{t("migration.noteWorlds")}</li>
        <li>{t("migration.noteState")}</li>
        <li>{t("migration.noteBackups")}</li>
      </ul>
      {open && (
        <Modal
          title={t("migration.dialogTitle")}
          onClose={() => {
            if (!exporting) setOpen(false);
          }}
          wide
        >
          {result ? (
            <>
              <div className="migration-success">
                <span className="metric-icon">
                  <Check size={16} />
                </span>
                <div>
                  <strong>{t("migration.done")}</strong>
                  <code>{result.path}</code>
                  <small>
                    {bytes(result.bytes)} ·{" "}
                    {t("import.filesUnit", { count: result.files })}
                    {result.includeBackups ? ` · ${t("migration.backupsIncluded")}` : ""}
                  </small>
                </div>
              </div>
              <p className="small muted">{t("migration.doneHint")}</p>
              <div className="modal-actions">
                <Button variant="primary" onClick={() => setOpen(false)}>
                  {t("common.close")}
                </Button>
              </div>
            </>
          ) : (
            <>
              <p className="muted">{t("migration.dialogDesc")}</p>
              {(active || info?.running) && (
                <p className="warning-text">
                  <TriangleAlert size={14} />
                  {t("migration.warnRunning")}
                </p>
              )}
              <Field label={t("migration.destination")}>
                <div className="row import-picker">
                  <input
                    autoFocus
                    value={path}
                    placeholder={t("migration.pathPlaceholder")}
                    disabled={exporting}
                    onChange={(e) => setPath(e.target.value)}
                  />
                  <Button
                    busy={busy.has("migration-pick")}
                    disabled={exporting}
                    onClick={() => void browse()}
                  >
                    <FolderOpen size={16} />
                    {t("migration.browse")}
                  </Button>
                </div>
              </Field>
              <label className="check-row">
                <input
                  type="checkbox"
                  checked={includeBackups}
                  disabled={exporting}
                  onChange={(e) => setIncludeBackups(e.target.checked)}
                />
                <span>
                  {t("migration.includeBackups", {
                    size: info ? bytes(info.backupsBytes) : "",
                  })}
                  <small>{t("migration.includeBackupsHint")}</small>
                </span>
              </label>
              <ul className="import-notes">
                <li>{t("migration.noteExcluded")}</li>
                <li>{t("migration.noteSecrets")}</li>
                <li>{t("migration.noteNoEncryption")}</li>
              </ul>
              {progress && (
                <div className="download">
                  <div>
                    <span>{t("migration.progress")}</span>
                    <strong>{progress.totalBytes ? `${percent}%` : ""}</strong>
                  </div>
                  <progress
                    max={progress.totalBytes || undefined}
                    value={
                      progress.totalBytes ? progress.bytes : undefined
                    }
                  />
                  <small>
                    {bytes(progress.bytes)}
                    {progress.totalBytes ? ` / ${bytes(progress.totalBytes)}` : ""}
                    {" · "}
                    {t("import.progressFiles", {
                      done: progress.files,
                      total: progress.totalFiles,
                    })}
                  </small>
                </div>
              )}
              {!path.trim().toLowerCase().endsWith(".zip") &&
                path.trim().length > 0 && (
                  <p className="warning-text">
                    <TriangleAlert size={14} />
                    {t("migration.needZip")}
                  </p>
                )}
              <div className="modal-actions">
                <Button disabled={exporting} onClick={() => setOpen(false)}>
                  {t("common.cancel")}
                </Button>
                <Button
                  variant="primary"
                  disabled={!canExport}
                  busy={exporting}
                  onClick={() => void submit()}
                >
                  <Archive size={16} />
                  {exporting ? t("migration.exporting") : t("migration.start")}
                </Button>
              </div>
              <p className="small muted import-hint">
                <ShieldCheck size={14} />
                {t("migration.hint")}
              </p>
            </>
          )}
        </Modal>
      )}
    </section>
  );
}
