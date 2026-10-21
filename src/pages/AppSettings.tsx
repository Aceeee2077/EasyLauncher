import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  FolderOpen,
  Globe,
  Monitor,
  Cpu,
  Info,
  Download,
  Languages,
  PackagePlus,
  RefreshCw,
  ScrollText,
  Shield,
  Trash2,
  TriangleAlert,
} from "lucide-react";
import { Button, Field, GuideLink, Modal, bytes, date } from "../components/ui";
import { brand, call, desktop } from "../services/api";
import { applyGlassOpacity } from "../services/appearance";
import { useI18n } from "../i18n";
import type {
  AuditEntry,
  JavaProgress,
  JavaRuntime,
  ManagedJava,
  SecretStatus,
  Settings,
  SystemInfo,
  UpdateStatus,
} from "../types";
import type { TaskProps } from "../utils/task";

const MAJORS = [8, 17, 21, 25] as const;

export function AppSettings({
  run,
  busy,
  info,
  onAppearance,
}: TaskProps & {
  info: SystemInfo | null;
  onAppearance: (a: Settings["appearance"]) => void;
}) {
  const { t, language, locale, setLanguage } = useI18n();
  const [settings, setSettings] = useState<Settings>({
    appearance: "light",
    language: "zh-CN",
    javaPath: null,
    downloadSource: "official",
    proxy: null,
    readOnly: false,
    assistantEndpoint: null,
    assistantModel: null,
    updateChannel: null,
    autoCheckUpdates: true,
    glassOpacity: 1,
    serverStorage: null,
    autoJoin: false,
  });
  const [java, setJava] = useState<JavaRuntime[]>([]);
  const [managed, setManaged] = useState<ManagedJava[]>([]);
  const [progress, setProgress] = useState<JavaProgress | null>(null);
  const [installing, setInstalling] = useState<number | null>(null);
  const [removing, setRemoving] = useState<ManagedJava | null>(null);
  const [audit, setAudit] = useState<AuditEntry[]>([]);
  const [secrets, setSecrets] = useState<SecretStatus | null>(null);
  const [clearingAudit, setClearingAudit] = useState(false);
  const [message, setMessage] = useState("");
  const [updateStatus, setUpdateStatus] = useState<UpdateStatus | null>(null);
  const [storageTarget, setStorageTarget] = useState<string | null>(null);
  const [storageMoved, setStorageMoved] = useState("");

  const reload = async () => setManaged(await call("list_managed_java"));
  const reloadAudit = async () =>
    setAudit(await call("audit_list", { limit: 50 }));
  const reloadSecrets = async () => setSecrets(await call("secret_status"));

  useEffect(() => {
    void run(`getSettings-${crypto.randomUUID()}`, async () =>
      setSettings(await call("get_settings")),
    );
    void run("listManaged", reload);
    void run("loadAudit", reloadAudit);
    void run("loadSecrets", reloadSecrets);
  }, []);

  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    const subscription = listen<JavaProgress>("java-progress", (e) => {
      if (alive) setProgress(e.payload);
    });
    return () => {
      alive = false;
      void subscription.then((f) => f());
    };
  }, []);

  const stage = (value?: string) => {
    switch (value) {
      case "resolving":
        return t("java.stage.resolving");
      case "downloading":
        return t("java.stage.downloading");
      case "verifying":
        return t("java.stage.verifying");
      case "extracting":
        return t("java.stage.extracting");
      case "failed":
        return t("java.stage.failed");
      default:
        return "";
    }
  };

  const changeLanguage = (next: Settings["language"]) => {
    setLanguage(next);
    const updated = { ...settings, language: next };
    setSettings(updated);
    void run("saveLanguage", () =>
      call("save_settings", { settings: updated }),
    );
  };

  const installJava = (major: number) =>
    void run(`installJava-${major}`, async () => {
      setInstalling(major);
      setProgress(null);
      try {
        await call("install_java", { major });
        await reload();
        setMessage(t("settings.java.installed", { major }));
      } finally {
        setInstalling(null);
        setProgress(null);
      }
    });

  const removeJava = (item: ManagedJava) =>
    void run("removeJava", async () => {
      await call("delete_java", { id: item.id, confirmed: true });
      setRemoving(null);
      await reload();
      setMessage(t("settings.java.removed", { major: item.major }));
    });

  const toggleReadOnly = () =>
    void run("toggleReadOnly", async () => {
      const updated = { ...settings, readOnly: !settings.readOnly };
      await call("save_settings", { settings: updated });
      setSettings(updated);
      setMessage(t("settings.saved"));
      await reloadAudit();
    });

  const clearAudit = () =>
    void run("clearAudit", async () => {
      await call("audit_clear");
      setClearingAudit(false);
      await reloadAudit();
      setMessage(t("settings.security.cleared"));
    });

  const checkUpdate = () =>
    void run("updateCheck", async () => {
      const normalized = {
        ...settings,
        updateChannel: settings.updateChannel?.trim() || null,
      };
      await call("save_settings", { settings: normalized });
      setSettings(normalized);
      setUpdateStatus(await call("update_check"));
    });

  const downloadUpdate = () =>
    void run("updateDownload", async () => {
      if (!updateStatus?.latestVersion) return;
      setUpdateStatus(
        await call("update_download", {
          version: updateStatus.latestVersion,
        }),
      );
    });

  const openUpdate = () =>
    void run("updateOpen", async () => {
      if (!updateStatus?.latestVersion) return;
      await call("update_open", { version: updateStatus.latestVersion });
    });

  return (
    <div className="settings-stack app-settings">
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Languages size={18} />
              {t("language.title")}
            </h2>
            <p>{t("language.description")}</p>
          </div>
        </div>
        <div className="segmented">
          {(["system", "zh-CN", "en"] as const).map((value) => (
            <button
              key={value}
              className={language === value ? "selected" : ""}
              onClick={() => changeLanguage(value)}
            >
              {t(`language.${value}`)}
            </button>
          ))}
        </div>
        <p className="small muted">
          {t("language.detected", { locale })} · {t("language.hint")}
        </p>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Monitor size={18} />
              {t("settings.appearance.title")}
            </h2>
            <p>{t("settings.appearance.desc")}</p>
          </div>
        </div>
        <div className="segmented">
          {(["system", "light", "dark"] as const).map((a) => (
            <button
              key={a}
              className={settings.appearance === a ? "selected" : ""}
              onClick={() => {
                setSettings((s) => ({ ...s, appearance: a }));
                onAppearance(a);
              }}
            >
              {t(`settings.appearance.${a}`)}
            </button>
          ))}
        </div>
        <Field
          label={t("settings.appearance.opacity")}
          hint={t("settings.appearance.opacityHint")}
        >
          <div className="slider-row">
            <input
              className="glass-slider"
              type="range"
              min={0}
              max={80}
              step={5}
              aria-label={t("settings.appearance.opacity")}
              value={Math.round((1 - settings.glassOpacity) * 100)}
              onChange={(e) => {
                const next = 1 - Number(e.target.value) / 100;
                setSettings((s) => ({ ...s, glassOpacity: next }));
                applyGlassOpacity(next);
              }}
            />
            <span className="slider-value">
              {t("settings.appearance.opacityValue", {
                value: Math.round((1 - settings.glassOpacity) * 100),
              })}
            </span>
          </div>
        </Field>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Cpu size={18} />
              {t("settings.java.title")}
            </h2>
            <p>{t("settings.java.desc")}</p>
          </div>
          <Button
            busy={busy.has("detectSettings")}
            onClick={() =>
              void run("detectSettings", async () =>
                setJava(
                  await call("detect_java", {
                    custom: settings.javaPath || undefined,
                  }),
                ),
              )
            }
          >
            {t("settings.java.detect")}
          </Button>
        </div>
        <Field label={t("settings.java.preferred")}>
          <input
            placeholder={t("settings.java.preferredPlaceholder")}
            value={settings.javaPath || ""}
            onChange={(e) =>
              setSettings((s) => ({ ...s, javaPath: e.target.value || null }))
            }
          />
        </Field>
        {java.map((j) => (
          <div className="detected" key={j.path}>
            <strong>Java {j.major}</strong>
            <code>{j.path}</code>
            <Button
              variant="ghost"
              onClick={() => setSettings((s) => ({ ...s, javaPath: j.path }))}
            >
              {t("settings.java.usePath")}
            </Button>
          </div>
        ))}
        <GuideLink
          onClick={() =>
            void run("guide", () => call("open_guide", { kind: "java" }))
          }
        >
          {t("settings.java.guide")}
        </GuideLink>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <PackagePlus size={18} />
              {t("settings.java.managedTitle")}
            </h2>
            <p>{t("settings.java.managedDesc")}</p>
          </div>
        </div>
        <div className="row">
          {MAJORS.map((major) => (
            <Button
              key={major}
              busy={installing === major}
              disabled={installing !== null}
              onClick={() => installJava(major)}
            >
              {t("settings.java.install", { major })}
            </Button>
          ))}
        </div>
        {installing !== null && progress && (
          <div className="download">
            <div>
              <span>
                {stage(progress.stage)}
                {progress.attempt > 1
                  ? ` · ${t("java.progress.attempt", {
                      n: progress.attempt,
                    })}`
                  : ""}
              </span>
              <strong>
                {progress.totalBytes
                  ? `${Math.min(
                      100,
                      Math.floor(
                        (progress.downloadedBytes / progress.totalBytes) * 100,
                      ),
                    )}%`
                  : ""}
              </strong>
            </div>
            <progress
              max={progress.totalBytes || undefined}
              value={
                progress.totalBytes ? progress.downloadedBytes : undefined
              }
            />
            <small>
              {bytes(progress.downloadedBytes)}
              {progress.totalBytes
                ? ` / ${bytes(progress.totalBytes)}`
                : ""}
            </small>
          </div>
        )}
        {managed.length ? (
          managed.map((item) => (
            <div className="detected" key={item.id}>
              <strong>
                {t("settings.java.major", { major: item.major })}
                <span className="tag">{t("settings.java.managedTag")}</span>
              </strong>
              <code>{item.path}</code>
              <span className="small muted">
                {item.version} · {bytes(item.sizeBytes)} ·{" "}
                {t("settings.java.installedAt", {
                  date: date(item.installedAt),
                })}
              </span>
              <Button variant="ghost" onClick={() => setRemoving(item)}>
                <Trash2 size={15} />
                {t("settings.java.remove")}
              </Button>
            </div>
          ))
        ) : (
          <p className="small muted">{t("settings.java.managedEmpty")}</p>
        )}
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <FolderOpen size={18} />
              {t("settings.storage.title")}
            </h2>
            <p>{t("settings.storage.desc")}</p>
          </div>
        </div>
        <code className="path-block">
          {info?.storagePath || t("settings.storage.unavailable")}
        </code>
        <div className="row">
          <Button
            onClick={() =>
              void run("folder", () => call("open_folder", { kind: "storage" }))
            }
          >
            {t("settings.storage.open")}
          </Button>
          <Button
            busy={busy.has("changeStorage")}
            onClick={() =>
              void run("changeStorage", async () => {
                const path = await call("pick_storage_folder");
                if (path) setStorageTarget(path);
              })
            }
          >
            {t("settings.storage.change")}
          </Button>
        </div>
        <p className="small muted">{t("settings.storage.changeHint")}</p>
        {storageMoved && (
          <p className="small muted" role="status">
            {t("settings.storage.moved", { path: storageMoved })}
          </p>
        )}
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Globe size={18} />
              {t("settings.network.title")}
            </h2>
            <p>{t("settings.network.desc")}</p>
          </div>
        </div>
        <label className="inline-check">
          <input
            type="checkbox"
            checked={settings.autoJoin}
            onChange={(e) =>
              setSettings((s) => ({ ...s, autoJoin: e.target.checked }))
            }
          />
          <strong>{t("settings.network.autoJoin")}</strong>
        </label>
        <p className="small muted">{t("settings.network.autoJoinHint")}</p>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Download size={18} />
              {t("settings.downloads.title")}
            </h2>
            <p>{t("settings.downloads.desc")}</p>
          </div>
        </div>
        <code className="path-block">
          {info?.downloadPath || t("settings.downloads.desc")}
        </code>
        <div className="field">
          <span>{t("settings.downloads.source")}</span>
          <div className="segmented">
            {(["official", "china"] as const).map((value) => (
              <button
                key={value}
                className={settings.downloadSource === value ? "selected" : ""}
                onClick={() =>
                  setSettings((s) => ({ ...s, downloadSource: value }))
                }
              >
                {t(`settings.downloads.${value}`)}
              </button>
            ))}
          </div>
        </div>
        {settings.downloadSource === "china" && (
          <p className="small muted">
            {t("settings.downloads.chinaHint")}
            <br />
            {t("settings.downloads.mirrorCredit")}
          </p>
        )}
        <Field
          label={t("settings.downloads.proxy")}
          hint={t("settings.downloads.proxyHint")}
        >
          <input
            placeholder={t("settings.downloads.proxyPlaceholder")}
            value={settings.proxy || ""}
            onChange={(e) =>
              setSettings((s) => ({ ...s, proxy: e.target.value || null }))
            }
          />
        </Field>
        <p className="small muted">{t("settings.downloads.incomplete")}</p>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Shield size={18} />
              {t("settings.security.title")}
            </h2>
            <p>{t("settings.security.desc")}</p>
          </div>
        </div>
        <label className="inline-check">
          <input
            type="checkbox"
            checked={settings.readOnly}
            disabled={busy.has("toggleReadOnly")}
            onChange={toggleReadOnly}
          />
          <strong>{t("settings.security.readOnly")}</strong>
        </label>
        <p className="small muted">{t("settings.security.readOnlyHint")}</p>
        <div className="section-title">
          <div>
            <h3>
              <ScrollText size={15} />
              {t("settings.security.audit")}
            </h3>
            <p>{t("settings.security.auditDesc")}</p>
          </div>
          <div className="row">
            <Button
              busy={busy.has("loadAudit")}
              onClick={() => void run("loadAudit", reloadAudit)}
            >
              <RefreshCw size={15} />
              {t("settings.security.refresh")}
            </Button>
            <Button
              variant="ghost"
              disabled={settings.readOnly || !audit.length}
              onClick={() => setClearingAudit(true)}
            >
              <Trash2 size={15} />
              {t("settings.security.clear")}
            </Button>
          </div>
        </div>
        {audit.length ? (
          <div className="alert-list">
            {audit.map((entry, index) => (
              <article
                className="alert-item"
                key={`${entry.at}-${entry.action}-${index}`}
              >
                <span className="alert-icon">
                  <ScrollText size={15} />
                </span>
                <div>
                  <strong>
                    <code>{entry.action}</code>
                    {entry.serverName ? ` · ${entry.serverName}` : ""}
                  </strong>
                  <small>{entry.detail}</small>
                </div>
                <time>{date(entry.at)}</time>
              </article>
            ))}
          </div>
        ) : (
          <p className="small muted">{t("settings.security.empty")}</p>
        )}
        <h3>{t("settings.security.secrets")}</h3>
        {secrets?.available ? (
          secrets.names.length ? (
            <div className="row">
              {secrets.names.map((name) => (
                <code className="tag" key={name}>
                  {name}
                </code>
              ))}
            </div>
          ) : (
            <p className="small muted">{t("settings.security.secretsEmpty")}</p>
          )
        ) : (
          <p className="small muted">
            {t("settings.security.secretsUnsupported")}
          </p>
        )}
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Download size={18} />
              {t("settings.update.title")}
            </h2>
            <p>{t("settings.update.desc")}</p>
          </div>
          <Button busy={busy.has("updateCheck")} onClick={checkUpdate}>
            <RefreshCw size={15} />
            {t("settings.update.check")}
          </Button>
        </div>
        <Field
          label={t("settings.update.channel")}
          hint={t("settings.update.channelHint")}
        >
          <input
            placeholder={t("settings.update.channelPlaceholder")}
            value={settings.updateChannel || ""}
            onChange={(e) =>
              setSettings((s) => ({
                ...s,
                updateChannel: e.target.value || null,
              }))
            }
          />
        </Field>
        <label className="inline-check">
          <input
            type="checkbox"
            checked={settings.autoCheckUpdates}
            onChange={(e) =>
              setSettings((s) => ({
                ...s,
                autoCheckUpdates: e.target.checked,
              }))
            }
          />
          <strong>{t("settings.update.autoCheck")}</strong>
        </label>
        <div className="update-status">
          <p className="small muted">
            {t("settings.update.current", {
              version: updateStatus?.currentVersion ?? brand.version,
            })}
          </p>
          {updateStatus &&
            (!updateStatus.configured ? (
              <p className="small muted">
                {t("settings.update.notConfigured")}
              </p>
            ) : !updateStatus.available ? (
              <p className="success-text small">
                {t("settings.update.upToDate", {
                  version: updateStatus.currentVersion,
                })}
              </p>
            ) : (
              <>
                <div className="update-found">
                  <strong>
                    {t("settings.update.available", {
                      version: updateStatus.latestVersion || "",
                    })}
                  </strong>
                  {updateStatus.notes && <p>{updateStatus.notes}</p>}
                  {!updateStatus.asset && (
                    <p className="service-warning">
                      <TriangleAlert size={16} />
                      {t("settings.update.noAsset")}
                    </p>
                  )}
                </div>
                {updateStatus.asset && (
                  <div className="row">
                    {updateStatus.downloadedPath ? (
                      <>
                        <span className="success-text small">
                          {t("settings.update.downloaded")}
                        </span>
                        <Button
                          busy={busy.has("updateOpen")}
                          disabled={settings.readOnly}
                          onClick={openUpdate}
                        >
                          {t("settings.update.open")}
                        </Button>
                        <Button
                          variant="ghost"
                          busy={busy.has("updateDownload")}
                          disabled={settings.readOnly}
                          onClick={downloadUpdate}
                        >
                          {t("settings.update.downloadAgain")}
                        </Button>
                      </>
                    ) : (
                      <Button
                        busy={busy.has("updateDownload")}
                        disabled={settings.readOnly}
                        onClick={downloadUpdate}
                      >
                        {t("settings.update.download", {
                          size: bytes(updateStatus.asset.size),
                        })}
                      </Button>
                    )}
                  </div>
                )}
                {settings.readOnly && (
                  <p className="small muted">
                    {t("settings.update.readOnly")}
                  </p>
                )}
                <p className="small muted">
                  {t("settings.update.openHint")}
                </p>
              </>
            ))}
        </div>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Info size={18} />
              {brand.name} {brand.version}
            </h2>
            <p>{t("settings.about.desc")}</p>
          </div>
          <Button
            onClick={() =>
              void run("folder", () => call("open_folder", { kind: "appLogs" }))
            }
          >
            {t("settings.about.logs")}
          </Button>
        </div>
      </section>
      <div className="row">
        <Button
          variant="primary"
          busy={busy.has("saveSettings")}
          onClick={() =>
            void run("saveSettings", async () => {
              await call("save_settings", { settings });
              setMessage(t("settings.saved"));
            })
          }
        >
          {t("settings.save")}
        </Button>
        {message && (
          <span className="success-text" role="status">
            {message}
          </span>
        )}
      </div>
      {storageTarget && (
        <Modal
          title={t("settings.storage.confirmTitle")}
          onClose={() => setStorageTarget(null)}
        >
          <p>
            {t("settings.storage.confirmBody", {
              from: info?.storagePath || "",
              to: storageTarget,
            })}
          </p>
          <div className="modal-actions">
            <Button onClick={() => setStorageTarget(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              busy={busy.has("moveStorage")}
              onClick={() =>
                void run("moveStorage", async () => {
                  const moved = await call("change_server_storage", {
                    path: storageTarget,
                  });
                  setStorageMoved(moved);
                  setSettings((s) => ({ ...s, serverStorage: moved }));
                  setStorageTarget(null);
                })
              }
            >
              {t("settings.storage.change")}
            </Button>
          </div>
        </Modal>
      )}
      {removing && (
        <Modal
          title={t("settings.java.removeTitle", { major: removing.major })}
          onClose={() => setRemoving(null)}
        >
          <p>
            {t("settings.java.removeBody", { version: removing.version })}
          </p>
          <div className="modal-actions">
            <Button onClick={() => setRemoving(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              busy={busy.has("removeJava")}
              onClick={() => removeJava(removing)}
            >
              {t("settings.java.remove")}
            </Button>
          </div>
        </Modal>
      )}
      {clearingAudit && (
        <Modal
          title={t("settings.security.clearTitle")}
          onClose={() => setClearingAudit(false)}
        >
          <p>{t("settings.security.clearBody")}</p>
          <div className="modal-actions">
            <Button onClick={() => setClearingAudit(false)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              busy={busy.has("clearAudit")}
              onClick={clearAudit}
            >
              {t("settings.security.clear")}
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}
