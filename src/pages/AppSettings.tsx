import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  FolderOpen,
  Monitor,
  Cpu,
  Info,
  Download,
  Languages,
  PackagePlus,
  Trash2,
} from "lucide-react";
import { Button, Field, GuideLink, Modal, bytes, date } from "../components/ui";
import { brand, call, desktop } from "../services/api";
import { useI18n } from "../i18n";
import type {
  JavaProgress,
  JavaRuntime,
  ManagedJava,
  Settings,
  SystemInfo,
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
  });
  const [java, setJava] = useState<JavaRuntime[]>([]);
  const [managed, setManaged] = useState<ManagedJava[]>([]);
  const [progress, setProgress] = useState<JavaProgress | null>(null);
  const [installing, setInstalling] = useState<number | null>(null);
  const [removing, setRemoving] = useState<ManagedJava | null>(null);
  const [message, setMessage] = useState("");

  const reload = async () => setManaged(await call("list_managed_java"));

  useEffect(() => {
    void run(`getSettings-${crypto.randomUUID()}`, async () =>
      setSettings(await call("get_settings")),
    );
    void run("listManaged", reload);
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
          <Button
            onClick={() =>
              void run("folder", () => call("open_folder", { kind: "storage" }))
            }
          >
            {t("settings.storage.open")}
          </Button>
        </div>
        <code className="path-block">
          {info?.storagePath || t("settings.storage.unavailable")}
        </code>
      </section>
      <section className="panel">
        <h2>
          <Download size={18} />
          {t("settings.downloads.title")}
        </h2>
        <p className="muted">
          {info?.downloadPath || t("settings.downloads.desc")}
        </p>
        <p className="small muted">{t("settings.downloads.incomplete")}</p>
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
    </div>
  );
}
