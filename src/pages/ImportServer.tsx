import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Archive,
  FolderOpen,
  FolderSearch,
  HardDrive,
  Layers,
  RefreshCw,
  ShieldCheck,
  TriangleAlert,
  Upload,
} from "lucide-react";
import { Button, Field, Modal, bytes, date } from "../components/ui";
import { call, desktop } from "../services/api";
import { useI18n, type MessageKey } from "../i18n";
import type {
  ArchivePreview,
  ImportPreview,
  ImportProgress,
  MigrationProgress,
  Server,
} from "../types";
type Run = <T>(key: string, fn: () => Promise<T>) => Promise<T | undefined>;
type Mode = "folder" | "archive";
function Progress({
  label,
  done,
  total,
  files,
  totalFiles,
}: {
  label: string;
  done: number;
  total: number;
  files: number;
  totalFiles: number;
}) {
  const percent = total ? Math.min(100, Math.floor((done / total) * 100)) : 0;
  return (
    <div className="download">
      <div>
        <span>{label}</span>
        <strong>{total ? `${percent}%` : ""}</strong>
      </div>
      <progress max={total || undefined} value={total ? done : undefined} />
      <small>
        {bytes(done)}
        {total ? ` / ${bytes(total)}` : ""}
        {" · "}
        {`${files} / ${totalFiles}`}
      </small>
    </div>
  );
}
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
  const [mode, setMode] = useState<Mode>("folder");
  const [path, setPath] = useState("");
  const [preview, setPreview] = useState<ImportPreview | null>(null);
  const [name, setName] = useState("");
  const [version, setVersion] = useState("");
  const [eula, setEula] = useState(false);
  const [progress, setProgress] = useState<ImportProgress | null>(null);
  const [archivePath, setArchivePath] = useState("");
  const [archive, setArchive] = useState<ArchivePreview | null>(null);
  const [archiveName, setArchiveName] = useState("");
  const [archiveVersion, setArchiveVersion] = useState("");
  const [archivePort, setArchivePort] = useState("25565");
  const [archiveEula, setArchiveEula] = useState(false);
  const [archiveProgress, setArchiveProgress] =
    useState<MigrationProgress | null>(null);
  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    const folder = listen<ImportProgress>("import-progress", (e) => {
      if (alive) setProgress(e.payload);
    });
    const archive = listen<MigrationProgress>("migration-progress", (e) => {
      if (alive) setArchiveProgress(e.payload);
    });
    return () => {
      alive = false;
      void folder.then((f) => f());
      void archive.then((f) => f());
    };
  }, []);
  const importing = busy.has("import") || busy.has("migration-import");
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
  const scanArchive = (target: string) =>
    run("migration-scan", async () => {
      setArchiveProgress(null);
      const result = await call("migration_preview_archive", {
        path: target.trim(),
      });
      setArchive(result);
      setArchiveName(result.name);
      setArchiveVersion(result.minecraftVersion);
      setArchivePort(String(result.suggestedPort));
      setArchiveEula(false);
    });
  const pickArchive = () =>
    run("migration-pick", async () => {
      const file = await call("migration_pick_archive");
      if (!file) return;
      setArchivePath(file);
      await scanArchive(file);
    });
  const submitArchive = () =>
    run("migration-import", async () => {
      setArchiveProgress(null);
      const port = Number(archivePort);
      const server = await call("migration_import_archive", {
        path: archivePath.trim(),
        name: archiveName.trim(),
        minecraftVersion: archiveVersion.trim(),
        port,
        eulaAccepted: archiveEula,
      });
      onImported(server);
    });
  const scanning = busy.has("import-scan");
  const archiveScanning = busy.has("migration-scan");
  const canImport =
    mode === "folder"
      ? !!preview &&
        !!path.trim() &&
        !!name.trim() &&
        !!version.trim() &&
        !!preview.jar &&
        eula
      : !!archive &&
        !!archivePath.trim() &&
        !!archiveName.trim() &&
        !!archiveVersion.trim() &&
        Number(archivePort) > 0 &&
        Number(archivePort) <= 65535 &&
        archiveEula;
  return (
    <Modal
      title={t("import.title")}
      onClose={() => {
        if (!importing) onClose();
      }}
      wide
    >
      <div className="import-tabs" role="tablist">
        <button
          role="tab"
          aria-selected={mode === "folder"}
          className={mode === "folder" ? "active" : ""}
          disabled={importing}
          onClick={() => setMode("folder")}
        >
          <FolderSearch size={15} />
          {t("import.tabFolder")}
        </button>
        <button
          role="tab"
          aria-selected={mode === "archive"}
          className={mode === "archive" ? "active" : ""}
          disabled={importing}
          onClick={() => setMode("archive")}
        >
          <Archive size={15} />
          {t("import.tabArchive")}
        </button>
      </div>
      {mode === "folder" ? (
        <>
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
                {preview ? (
                  <RefreshCw size={16} />
                ) : (
                  <FolderSearch size={16} />
                )}
                {preview ? t("import.rescan") : t("import.scan")}
              </Button>
            </div>
          </Field>
          {scanning && !preview && (
            <p className="mini-empty">{t("import.scanning")}</p>
          )}
          {preview && (
            <>
              <div className="eyebrow import-section">
                {t("import.summary")}
              </div>
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
                  <code>{t(`software.${preview.serverType}` as MessageKey)}</code>
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
                <Field
                  label={t("import.version")}
                  hint={t("import.versionHint")}
                >
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
            <Progress
              label={t("import.progress")}
              done={progress.copiedBytes}
              total={progress.totalBytes}
              files={progress.copiedFiles}
              totalFiles={progress.totalFiles}
            />
          )}
        </>
      ) : (
        <>
          <p className="muted">{t("import.archive.desc")}</p>
          <Field label={t("import.archive.path")}>
            <div className="row import-picker">
              <input
                autoFocus
                value={archivePath}
                placeholder={t("import.archive.pathPlaceholder")}
                disabled={importing}
                onChange={(e) => {
                  setArchivePath(e.target.value);
                  setArchive(null);
                }}
                onKeyDown={(e) => {
                  if (e.key === "Enter" && archivePath.trim() && !archiveScanning) {
                    void scanArchive(archivePath);
                  }
                }}
              />
              <Button
                busy={busy.has("migration-pick")}
                disabled={importing}
                onClick={() => void pickArchive()}
              >
                <FolderOpen size={16} />
                {t("import.archive.pick")}
              </Button>
              <Button
                variant="primary"
                busy={archiveScanning}
                disabled={!archivePath.trim() || importing}
                onClick={() => void scanArchive(archivePath)}
              >
                {archive ? <RefreshCw size={16} /> : <FolderSearch size={16} />}
                {archive ? t("import.rescan") : t("import.archive.scan")}
              </Button>
            </div>
          </Field>
          {archiveScanning && !archive && (
            <p className="mini-empty">{t("import.scanning")}</p>
          )}
          {archive && (
            <>
              <div className="eyebrow import-section">
                {t("import.summary")}
              </div>
              <div className="net-grid">
                <article className="net-card">
                  <span className="metric-icon">
                    <Layers size={16} />
                  </span>
                  <small>{t("import.version")}</small>
                  <code>{archive.minecraftVersion}</code>
                  <span className="net-badge">
                    {t(`software.${archive.serverType}` as MessageKey)}
                  </span>
                </article>
                <article className="net-card">
                  <span className="metric-icon">
                    <Archive size={16} />
                  </span>
                  <small>{t("import.archive.unpacked")}</small>
                  <code>{bytes(archive.sizeBytes)}</code>
                  <span className="net-badge">
                    {t("import.filesUnit", { count: archive.files })}
                  </span>
                </article>
                <article className="net-card">
                  <span className="metric-icon">
                    <HardDrive size={16} />
                  </span>
                  <small>{t("import.archive.fileSize")}</small>
                  <code>{bytes(archive.archiveBytes)}</code>
                  <span className="net-badge">
                    {t("import.archive.appVersion", {
                      version: archive.appVersion,
                    })}
                  </span>
                </article>
                <article className="net-card">
                  <span className="metric-icon">
                    <ShieldCheck size={16} />
                  </span>
                  <small>{t("import.archive.created")}</small>
                  <code>{date(archive.createdAt)}</code>
                  <span className="net-badge">
                    {t("import.archive.java", { major: archive.requiredJava })}
                  </span>
                </article>
              </div>
              {archive.warnings.length > 0 && (
                <div className="import-warnings">
                  {archive.warnings.map((warning) => (
                    <p className="warning-text" key={warning}>
                      <TriangleAlert size={14} />
                      {t(`import.warn.${warning}` as MessageKey, {
                        port: archive.suggestedPort,
                      })}
                    </p>
                  ))}
                </div>
              )}
              <div className="form-grid import-form">
                <Field label={t("import.name")}>
                  <input
                    maxLength={64}
                    value={archiveName}
                    placeholder={t("import.namePlaceholder")}
                    disabled={importing}
                    onChange={(e) => setArchiveName(e.target.value)}
                  />
                </Field>
                <Field label={t("import.version")} hint={t("import.versionHint")}>
                  <input
                    value={archiveVersion}
                    placeholder="1.20.4"
                    disabled={importing}
                    onChange={(e) => setArchiveVersion(e.target.value)}
                  />
                </Field>
                <Field
                  label={t("import.port")}
                  hint={t("import.archive.portHint")}
                >
                  <input
                    type="number"
                    min={1}
                    max={65535}
                    value={archivePort}
                    disabled={importing}
                    onChange={(e) => setArchivePort(e.target.value)}
                  />
                </Field>
              </div>
              <label className="check-row">
                <input
                  type="checkbox"
                  checked={archiveEula}
                  disabled={importing}
                  onChange={(e) => setArchiveEula(e.target.checked)}
                />
                <span>
                  {t("import.eula")}
                  <small>{t("import.archive.eulaHint")}</small>
                </span>
              </label>
              <ul className="import-notes">
                <li>{t("import.archive.noteScope")}</li>
                <li>{t("import.archive.noteState")}</li>
                <li>{t("import.archive.noteVerify")}</li>
                <li>{t("import.noteJava")}</li>
              </ul>
            </>
          )}
          {archiveProgress && (
            <Progress
              label={t("import.archive.progress")}
              done={archiveProgress.bytes}
              total={archiveProgress.totalBytes}
              files={archiveProgress.files}
              totalFiles={archiveProgress.totalFiles}
            />
          )}
        </>
      )}
      <div className="modal-actions">
        <Button disabled={importing} onClick={onClose}>
          {t("common.cancel")}
        </Button>
        <Button
          variant="primary"
          disabled={!canImport}
          busy={importing}
          onClick={() => void (mode === "folder" ? submit() : submitArchive())}
        >
          <Upload size={16} />
          {importing
            ? t("import.importing")
            : mode === "folder"
              ? t("import.start")
              : t("import.archive.start")}
        </Button>
      </div>
      <p className="small muted import-hint">
        <ShieldCheck size={14} />
        {mode === "folder"
          ? !preview
            ? t("import.needFolder")
            : !preview.jar
              ? t("import.warn.noJar")
              : !eula
                ? t("import.needEula")
                : t("import.noteCopy")
          : !archive
            ? t("import.archive.needFile")
            : !archiveEula
              ? t("import.needEula")
              : t("import.archive.ready")}
      </p>
    </Modal>
  );
}
