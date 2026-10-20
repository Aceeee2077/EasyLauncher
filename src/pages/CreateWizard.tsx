import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  ArrowLeft,
  ArrowRight,
  Check,
  Cpu,
  Download,
  ShieldCheck,
  Layers,
  Leaf,
  Box,
  Gem,
} from "lucide-react";
import { Button, Field, Modal, Step, GuideLink, bytes } from "../components/ui";
import { call, desktop } from "../services/api";
import { useI18n } from "../i18n";
import type {
  CreateServer,
  JavaProgress,
  JavaRuntime,
  Server,
  ServerType,
  SystemInfo,
  Version,
} from "../types";
type Run = <T>(key: string, fn: () => Promise<T>) => Promise<T | undefined>;
export function CreateWizard({
  onClose,
  onCreated,
  info,
  run,
  busy,
}: {
  onClose: () => void;
  onCreated: (s: Server) => void;
  info: SystemInfo | null;
  run: Run;
  busy: Set<string>;
}) {
  const { t } = useI18n();
  const [step, setStep] = useState(0);
  const [versions, setVersions] = useState<Version[]>([]);
  const [loading, setLoading] = useState(false);
  const [required, setRequired] = useState<number | null>(null);
  const [runtimes, setRuntimes] = useState<JavaRuntime[]>([]);
  const [checked, setChecked] = useState("");
  const [installing, setInstalling] = useState<number | null>(null);
  const [progress, setProgress] = useState<JavaProgress | null>(null);
  const [installed, setInstalled] = useState("");
  const [snapshots, setSnapshots] = useState(false);
  const [form, setForm] = useState<CreateServer>({
    name: "",
    minecraftVersion: "",
    serverType: "vanilla",
    memoryMinMb: 1024,
    memoryMaxMb: info?.recommendedMb || 2048,
    port: 25565,
    maxPlayers: 20,
    gameMode: "survival",
    difficulty: "normal",
    onlineMode: true,
    javaPath: "",
    eulaAccepted: false,
  });
  function change<K extends keyof CreateServer>(k: K, v: CreateServer[K]) {
    setForm((f) => ({ ...f, [k]: v }));
    if (k === "javaPath") setChecked("");
  }
  useEffect(() => {
    let live = true;
    setLoading(true);
    setVersions([]);
    change("minecraftVersion", "");
    void run(`versions-${crypto.randomUUID()}`, async () => {
      const v = await call("list_versions", {
        software: form.serverType,
        snapshots,
      });
      if (live) {
        setVersions(v);
        const first = v.find((x) => x.type === "release") ?? v[0];
        setForm((f) => ({
          ...f,
          minecraftVersion: v.some((x) => x.id === f.minecraftVersion)
            ? f.minecraftVersion
            : first?.id || "",
        }));
      }
    }).finally(() => {
      if (live) setLoading(false);
    });
    return () => {
      live = false;
    };
  }, [form.serverType, snapshots]);
  useEffect(() => {
    let live = true;
    setRequired(null);
    setChecked("");
    setInstalled("");
    if (form.minecraftVersion)
      void run(`requirement-${crypto.randomUUID()}`, async () => {
        const r = await call("java_requirement", {
          version: form.minecraftVersion,
          software: form.serverType,
        });
        if (live) setRequired(r);
      });
    return () => {
      live = false;
    };
  }, [form.minecraftVersion, form.serverType]);
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
  const detect = () =>
    run("detect", async () => {
      const js = await call("detect_java", {});
      setRuntimes(js);
      const best = js.find((j) => j.major === required);
      if (best) {
        setForm((f) => ({ ...f, javaPath: best.path }));
        setChecked(best.path);
      }
    });
  useEffect(() => {
    if (step === 2 && required) void detect();
  }, [step, required]);
  const memoryInvalid =
    !!info &&
    (form.memoryMaxMb >
      Math.min(
        info.totalMb - 1024,
        info.totalMb * 0.8,
        info.availableMb - 512,
      ) ||
      form.memoryMinMb > form.memoryMaxMb ||
      form.memoryMinMb < 256);
  const canNext =
    step === 0
      ? !!form.name.trim() && !!form.minecraftVersion && !loading
      : step === 1
        ? !memoryInvalid &&
          form.port > 0 &&
          form.port < 65536 &&
          form.maxPlayers > 0
        : step === 2
          ? !!required && checked === form.javaPath
          : false;
  const creating = busy.has("create");
  const compatible = runtimes.some((j) => j.major === required);
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
  const installJava = () => {
    if (!required) return;
    const major = required;
    void run("installJava", async () => {
      setInstalling(major);
      setProgress(null);
      setInstalled("");
      try {
        await call("install_java", { major });
        await detect();
        setInstalled(t("wizard.javaInstalled", { major }));
      } finally {
        setInstalling(null);
        setProgress(null);
      }
    });
  };
  return (
    <Modal
      title={t("wizard.title")}
      onClose={() => {
        if (!creating) onClose();
      }}
      wide
    >
      <div className="wizard-steps">
        {(
          [
            "wizard.step.server",
            "wizard.step.resources",
            "wizard.step.java",
            "wizard.step.agreement",
          ] as const
        ).map((key, i) => (
          <Step
            key={key}
            n={i + 1}
            label={t(key)}
            active={i === step}
            done={step > i}
          />
        ))}
      </div>
      <div className="wizard-body">
        {step === 0 && (
          <>
            <div className="eyebrow">{t("wizard.step1.eyebrow")}</div>
            <h2>{t("wizard.step1.title")}</h2>
            <p className="muted">{t("wizard.step1.desc")}</p>
            <Field label={t("wizard.name")}>
              <input
                autoFocus
                maxLength={64}
                placeholder={t("wizard.namePlaceholder")}
                value={form.name}
                onChange={(e) => change("name", e.target.value)}
              />
            </Field>
            <Field label={t("wizard.software")}>
              <div className="software-options">
                {(
                  [
                    [
                      "vanilla",
                      "software.vanilla",
                      "software.vanillaDesc",
                      Box,
                    ],
                    ["paper", "software.paper", "software.paperDesc", Leaf],
                    ["purpur", "software.purpur", "software.purpurDesc", Gem],
                    ["fabric", "software.fabric", "software.fabricDesc", Layers],
                  ] as const
                ).map(([type, title, desc, Icon]) => (
                  <button
                    key={type}
                    className={`software-option ${form.serverType === type ? "selected" : ""}`}
                    onClick={() => change("serverType", type as ServerType)}
                  >
                    <Icon size={22} />
                    <strong>{t(title)}</strong>
                    <small>{t(desc)}</small>
                  </button>
                ))}
              </div>
            </Field>
            <Field label={t("wizard.version")} hint={t("wizard.versionHint")}>
              <select
                disabled={loading}
                value={form.minecraftVersion}
                onChange={(e) => change("minecraftVersion", e.target.value)}
              >
                <option value="">
                  {loading
                    ? t("wizard.loadingVersions")
                    : t("wizard.selectRelease")}
                </option>
                {versions.map((v) => (
                  <option key={v.id} value={v.id}>
                    {v.type === "snapshot"
                      ? `${v.id} · ${t("wizard.snapshot")}`
                      : v.id}
                  </option>
                ))}
              </select>
            </Field>
            {(form.serverType === "vanilla" ||
              form.serverType === "fabric") && (
              <label className="check-row">
                <input
                  type="checkbox"
                  checked={snapshots}
                  onChange={(e) => setSnapshots(e.target.checked)}
                />
                <span>
                  {t("wizard.showSnapshots")}{" "}
                  <small>{t("wizard.snapshotsHint")}</small>
                </span>
              </label>
            )}
            {!loading && !versions.length && (
              <Button
                onClick={() => {
                  void run("versionsRetry", async () => {
                    const v = await call("list_versions", {
                      software: form.serverType,
                      snapshots,
                    });
                    setVersions(v);
                    const first = v.find((x) => x.type === "release") ?? v[0];
                    change("minecraftVersion", first?.id || "");
                  });
                }}
              >
                {t("wizard.retryVersions")}
              </Button>
            )}
          </>
        )}
        {step === 1 && (
          <>
            <div className="eyebrow">{t("wizard.step2.eyebrow")}</div>
            <h2>{t("wizard.step2.title")}</h2>
            <div className="info-strip">
              <Cpu size={18} />
              {info
                ? t("wizard.memorySummary", {
                    total: (info.totalMb / 1024).toFixed(1),
                    available: (info.availableMb / 1024).toFixed(1),
                    recommended: info.recommendedMb,
                  })
                : t("wizard.readingMemory")}
            </div>
            <div className="form-grid">
              <Field label={t("wizard.memoryMin")}>
                <input
                  type="number"
                  min={256}
                  step={256}
                  value={form.memoryMinMb}
                  onChange={(e) => change("memoryMinMb", +e.target.value)}
                />
              </Field>
              <Field label={t("wizard.memoryMax")}>
                <input
                  type="number"
                  min={256}
                  step={256}
                  value={form.memoryMaxMb}
                  onChange={(e) => change("memoryMaxMb", +e.target.value)}
                />
              </Field>
              <Field label={t("wizard.maxPlayers")}>
                <input
                  type="number"
                  min={1}
                  max={1000}
                  value={form.maxPlayers}
                  onChange={(e) => change("maxPlayers", +e.target.value)}
                />
              </Field>
              <Field label={t("wizard.port")}>
                <input
                  type="number"
                  min={1}
                  max={65535}
                  value={form.port}
                  onChange={(e) => change("port", +e.target.value)}
                />
              </Field>
              <Field label={t("wizard.gameMode")}>
                <select
                  value={form.gameMode}
                  onChange={(e) => change("gameMode", e.target.value)}
                >
                  {(
                    ["survival", "creative", "adventure", "spectator"] as const
                  ).map((v) => (
                    <option key={v} value={v}>
                      {t(`value.${v}`)}
                    </option>
                  ))}
                </select>
              </Field>
              <Field label={t("wizard.difficulty")}>
                <select
                  value={form.difficulty}
                  onChange={(e) => change("difficulty", e.target.value)}
                >
                  {(["peaceful", "easy", "normal", "hard"] as const).map((v) => (
                    <option key={v} value={v}>
                      {t(`value.${v}`)}
                    </option>
                  ))}
                </select>
              </Field>
            </div>
            <label className="check-row">
              <input
                type="checkbox"
                checked={form.onlineMode}
                onChange={(e) => change("onlineMode", e.target.checked)}
              />
              <span>
                {t("wizard.onlineMode")} <small>{t("wizard.onlineModeHint")}</small>
              </span>
            </label>
            {memoryInvalid && (
              <p className="warning-text">{t("wizard.memoryWarning")}</p>
            )}
          </>
        )}
        {step === 2 && (
          <>
            <div className="eyebrow">{t("wizard.step3.eyebrow")}</div>
            <h2>{t("wizard.step3.title")}</h2>
            <p className="muted">
              {required
                ? t("wizard.javaRequirement", {
                    software: form.serverType,
                    version: form.minecraftVersion,
                    major: required,
                  })
                : t("wizard.javaResolving")}{" "}
              {t("wizard.javaCompatibility")}
            </p>
            <Button busy={busy.has("detect")} onClick={() => void detect()}>
              <Cpu size={16} />
              {t("wizard.detect")}
            </Button>
            <div className="runtime-list">
              {runtimes.map((j) => (
                <button
                  className={`runtime ${form.javaPath === j.path ? "selected" : ""}`}
                  key={j.path}
                  onClick={() => {
                    change("javaPath", j.path);
                    if (j.major === required) setChecked(j.path);
                  }}
                >
                  <span>
                    <strong>Java {j.major}</strong>
                    <small>{j.path}</small>
                  </span>
                  <span
                    className={j.major === required ? "success-text" : "muted"}
                  >
                    {j.major === required
                      ? t("wizard.compatible")
                      : t("wizard.needsJava", { major: required ?? "—" })}
                  </span>
                </button>
              ))}
            </div>
            <Field
              label={t("wizard.customJava")}
              hint={t("wizard.customJavaHint")}
            >
              <input
                value={form.javaPath}
                placeholder={t("wizard.javaPathPlaceholder")}
                onChange={(e) => change("javaPath", e.target.value)}
              />
            </Field>
            <div className="row">
              <Button
                disabled={!form.javaPath || !required}
                busy={busy.has("validate")}
                onClick={() =>
                  void run("validate", async () => {
                    await call("validate_java", {
                      path: form.javaPath,
                      required: required!,
                    });
                    setChecked(form.javaPath);
                  })
                }
              >
                {t("wizard.validate")}
              </Button>
              {checked === form.javaPath && checked && (
                <span className="success-text">
                  <Check size={16} />
                  {t("wizard.compatibleRuntime")}
                </span>
              )}
            </div>
            <div className="guidance">
              <strong>{t("wizard.javaMissing")}</strong>
              <p>{t("wizard.javaMissingDesc", { major: required ?? "—" })}</p>
              {required && !compatible && (
                <>
                  <Button
                    variant="primary"
                    busy={installing !== null}
                    disabled={installing !== null}
                    onClick={installJava}
                  >
                    <Download size={16} />
                    {installing !== null
                      ? t("wizard.javaInstalling", { major: installing })
                      : t("wizard.javaInstall", { major: required })}
                  </Button>
                  <p className="small muted">{t("wizard.javaInstallHint")}</p>
                </>
              )}
              {installing !== null && progress && (
                <div className="download">
                  <div>
                    <span>{stage(progress.stage)}</span>
                    <strong>
                      {progress.totalBytes
                        ? `${Math.min(
                            100,
                            Math.floor(
                              (progress.downloadedBytes /
                                progress.totalBytes) *
                                100,
                            ),
                          )}%`
                        : ""}
                    </strong>
                  </div>
                  <progress
                    max={progress.totalBytes || undefined}
                    value={
                      progress.totalBytes
                        ? progress.downloadedBytes
                        : undefined
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
              {installed && <span className="success-text">{installed}</span>}
              <GuideLink
                onClick={() =>
                  void run("guide", () => call("open_guide", { kind: "java" }))
                }
              >
                {t("wizard.openGuide")}
              </GuideLink>
            </div>
          </>
        )}
        {step === 3 && (
          <>
            <div className="agreement-icon">
              <ShieldCheck size={30} />
            </div>
            <h2>{t("wizard.step4.title")}</h2>
            <p className="muted">{t("wizard.step4.desc")}</p>
            <GuideLink
              onClick={() =>
                void run("guide", () => call("open_guide", { kind: "eula" }))
              }
            >
              {t("wizard.readEula")}
            </GuideLink>
            <div className="review-card">
              <strong>{form.name}</strong>
              <span>
                {t("wizard.review", {
                  type: form.serverType,
                  version: form.minecraftVersion,
                })}
              </span>
              <span>
                {t("wizard.reviewResources", {
                  memory: form.memoryMaxMb,
                  players: form.maxPlayers,
                  port: form.port,
                })}
              </span>
            </div>
            <label className="check-row agreement">
              <input
                type="checkbox"
                checked={form.eulaAccepted}
                onChange={(e) => change("eulaAccepted", e.target.checked)}
              />
              <span>
                {t("wizard.eulaAgree")}
                <small>{t("wizard.eulaHint")}</small>
              </span>
            </label>
            <p className="small muted">{t("wizard.installNote")}</p>
          </>
        )}
      </div>
      <div className="wizard-footer">
        <Button
          disabled={step === 0 || creating}
          variant="ghost"
          onClick={() => setStep((s) => s - 1)}
        >
          <ArrowLeft size={16} />
          {t("common.back")}
        </Button>
        <span className="small muted">
          {t("wizard.stepOf", { step: step + 1, total: 4 })}
        </span>
        {step < 3 ? (
          <Button
            variant="primary"
            disabled={!canNext}
            onClick={() => setStep((s) => s + 1)}
          >
            {t("common.continue")}
            <ArrowRight size={16} />
          </Button>
        ) : (
          <Button
            variant="primary"
            disabled={!form.eulaAccepted}
            busy={creating}
            onClick={() =>
              void run("create", async () => {
                const s = await call("create_server", { input: form });
                onCreated(s);
              })
            }
          >
            <Download size={16} />
            {t("wizard.createInstall")}
          </Button>
        )}
      </div>
    </Modal>
  );
}
