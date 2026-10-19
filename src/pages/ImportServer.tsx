import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  FolderOpen,
  FolderSearch,
  HardDrive,
  Layers,
  RefreshCw,
  ShieldCheck,
  TriangleAlert,
  Upload,
} from "lucide-react";
import { Button, Field, Modal, bytes } from "../components/ui";
import { call, desktop } from "../services/api";
import { useI18n, type MessageKey } from "../i18n";
import type { ImportPreview, ImportProgress, Server } from "../types";
type Run = <T>(key: string, fn: () => Promise<T>) => Promise<T | undefined>;
export function ImportServer({
  onClose,
  onImported,
  run,
  busy,
}: {
  onClose: () => void;
  onImported: (s: Server) => void;
  run: Run;
  busy: Set<string>;
}) {
  const { t } = useI18n();
  const [path, setPath] = useState("");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [name, setName] = useState("");
  const [version, setVersion] = useState("");
  const [eula, setEula] = useState(false);
  const [progress, setProgress] = useState<ImportProgress | null>(null);
  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    const subscription = listen<ImportProgress>("import-progress", (e) => {
      if (alive) setProgress(e.payload);
    });
    return () => {
      alive = false;
      void subscription.then((f) => f());
    };
  }, []);
  const importing = busy.has("import");
  const scanning = busy.has("import-scan");
  const scan = (target: string) =>
    run("import-scan", async () => {
      setProgress(null);
      const result = await call("preview_import", { path: target.trim() });
      setPreview(result);
      setName(result.suggestedName);
      setVersion(result.minecraftVersion);
      setEula(result.eulaAccepted);
    });
  const pick = () =>
    run("import-pick", async () => {
      const folder = await call("pick_server_folder");
      if (!folder) return;
      setPath(folder);
      await scan(folder);
    });
  const submit = () =>
    run("import", async () => {
      setProgress(null);
      const server = await call("import_server", {
        path: path.trim(),
        name: name.trim(),
        minecraftVersion: version.trim(),
        eulaAccepted: eula,
      });
      onImported(server);
    });
  const percent =
    progress && progress.totalBytes
      ? Math.min(
          100,
          Math.floor((progress.copiedBytes / progress.totalBytes) * 100),
        )
      : 0;
  const canImport =
    !!preview &&
    !!path.trim() &&
    !!name.trim() &&
    !!version.trim() &&
    !!preview.jar &&
    eula;
  return (
    <Modal
      title={t("import.title")}
      onClose={() => {
        if (!importing) onClose();
      }}
      wide
    >
      <p className="muted">{t("import.desc")}</p>
      <Field label={t("import.sourcePath")}>
        <div className="row import-picker">
          <input
            autoFocus
            value={path}
            placeholder={t("import.pathPlaceholder")}
            disabled={importing}
            onChange={(e) => {
              setPath(e.target.value);
              setPreview(null);
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && path.trim() && !scanning) {
                void scan(path);
              }
            }}
          />
          <Button
            busy={busy.has("import-pick")}
            disabled={importing}
            onClick={() => void pick()}
          >
            <FolderOpen size={16} />
            {t("import.pick")}
          </Button>
          <Button
            variant="primary"
            busy={scanning}
            disabled={!path.trim() || importing}
            onClick={() => void scan(path)}
          >
            {preview ? <RefreshCw size={16} /> : <FolderSearch size={16} />}
            {preview ? t("import.rescan") : t("import.scan")}
          </Button>
        </div>
      </Field>
      {scanning && !preview && (
        <p className="mini-empty">{t("import.scanning")}</p>
      )}
      {preview && (
        <>
          <div className="eyebrow import-section">{t("import.summary")}</div>
          <div className="net-grid">
            <article className="net-card">
              <span className="metric-icon">
                <Layers size={16} />
              </span>
              <small>{t("import.version")}</small>
              <code>{preview.minecraftVersion || t("common.unavailable")}</code>
              <span className="net-badge">
                {t(
                  `import.versionSource.${preview.versionSource}` as MessageKey,
                )}
              </span>
            </article>
            <article className="net-card">
              <span className="metric-icon">
                <FolderSearch size={16} />
              </span>
              <small>{t("import.type")}</small>
              <code>
                {t(`software.${preview.serverType}` as MessageKey)}
              </code>
              <span className="net-badge">{preview.port}</span>
            </article>
            <article className="net-card">
              <span className="metric-icon">
                <HardDrive size={16} />
              </span>
              <small>{t("import.jar")}</small>
              <code>{preview.jar || t("common.unavailable")}</code>
              {preview.jarRenameNeeded && (
                <span className="net-badge">{t("import.rename")}</span>
              )}
            </article>
            <article className="net-card">
              <span className="metric-icon">
                <HardDrive size={16} />
              </span>
              <small>{t("import.size")}</small>
              <code>{bytes(preview.sizeBytes)}</code>
              <span className="net-badge">
                {t("import.filesUnit", { count: preview.fileCount })}
              </span>
            </article>
          </div>
          {preview.warnings.length > 0 && (
            <div className="import-warnings">
              {preview.warnings.map((warning) => (
                <p className="warning-text" key={warning}>
                  <TriangleAlert size={14} />
                  {t(`import.warn.${warning}` as MessageKey)}
                </p>
              ))}
            </div>
          )}
          <div className="form-grid import-form">
            <Field label={t("import.name")}>
              <input
                maxLength={64}
                value={name}
                placeholder={t("import.namePlaceholder")}
                disabled={importing}
                onChange={(e) => setName(e.target.value)}
              />
            </Field>
            <Field label={t("import.version")} hint={t("import.versionHint")}>
              <input
                value={version}
                placeholder="1.20.4"
                disabled={importing}
                onChange={(e) => setVersion(e.target.value)}
              />
            </Field>
          </div>
          <label className="check-row">
            <input
              type="checkbox"
              checked={eula}
              disabled={importing}
              onChange={(e) => setEula(e.target.checked)}
            />
            <span>
              {t("import.eula")}
              <small>
                {preview.eulaAccepted
                  ? t("import.eulaFound")
                  : t("import.eulaHint")}
              </small>
            </span>
          </label>
          <ul className="import-notes">
            <li>{t("import.noteCopy")}</li>
            <li>{t("import.noteMeta")}</li>
            <li>{t("import.noteJava")}</li>
          </ul>
        </>
      )}
      {progress && (
        <div className="download">
          <div>
            <span>{t("import.progress")}</span>
            <strong>{progress.totalBytes ? `${percent}%` : ""}</strong>
          </div>
          <progress
            max={progress.totalBytes || undefined}
            value={progress.totalBytes ? progress.copiedBytes : undefined}
          />
          <small>
            {bytes(progress.copiedBytes)}
            {progress.totalBytes ? ` / ${bytes(progress.totalBytes)}` : ""}
            {" · "}
            {t("import.progressFiles", {
              done: progress.copiedFiles,
              total: progress.totalFiles,
            })}
          </small>
        </div>
      )}
      <div className="modal-actions">
        <Button disabled={importing} onClick={onClose}>
          {t("common.cancel")}
        </Button>
        <Button
          variant="primary"
          disabled={!canImport}
          busy={importing}
          onClick={() => void submit()}
        >
          <Upload size={16} />
          {importing ? t("import.importing") : t("import.start")}
        </Button>
      </div>
      <p className="small muted import-hint">
        <ShieldCheck size={14} />
        {!preview
          ? t("import.needFolder")
          : !preview.jar
            ? t("import.warn.noJar")
            : !eula
              ? t("import.needEula")
              : t("import.noteCopy")}
      </p>
    </Modal>
  );
}
