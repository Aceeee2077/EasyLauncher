import { useEffect, useState } from "react";
import {
  Save,
  FolderOpen,
  Settings2,
  Terminal,
  Copy,
  Check,
  Cpu,
  Layers,
  ArrowRight,
  Radio,
} from "lucide-react";
import { Button, Field, Modal } from "../components/ui";
import { call } from "../services/api";
import {
  idle,
  type JvmPreset,
  type LaunchInfo,
  type PropertyChange,
  type PropertyTemplate,
  type RconInfo,
  type ServerView,
} from "../types";
import type { TaskProps } from "../utils/task";
import { useI18n, type MessageKey } from "../i18n";
const numeric = [
  ["server-port", "serverSettings.port", 1, 65535],
  ["max-players", "serverSettings.maxPlayers", 1, 1000],
  ["spawn-protection", "serverSettings.spawnProtection", 0, 10000],
  ["view-distance", "serverSettings.viewDistance", 2, 32],
  ["simulation-distance", "serverSettings.simulationDistance", 2, 32],
] as const satisfies readonly (readonly [string, MessageKey, number, number])[];
const templates = ["survival", "creative", "performance"] as const;
export function ServerSettings({
  server,
  run,
  busy,
  onSaved,
}: TaskProps & { server: ServerView; onSaved: () => Promise<void> }) {
  const { t } = useI18n();
  const [values, setValues] = useState<Record<string, string> | null>(null);
  const [raw, setRaw] = useState("");
  const [advanced, setAdvanced] = useState(false);
  const [saved, setSaved] = useState(false);
  const [jars, setJars] = useState<string[]>([]);
  const [min, setMin] = useState(server.memoryMinMb);
  const [max, setMax] = useState(server.memoryMaxMb);
  const [java, setJava] = useState(server.javaPath || "");
  const [preset, setPreset] = useState<JvmPreset>(server.jvmPreset);
  const [flags, setFlags] = useState(server.jvmFlags.join("\n"));
  const [jvmSaved, setJvmSaved] = useState(false);
  const [launch, setLaunch] = useState<LaunchInfo | null>(null);
  const [copied, setCopied] = useState(false);
  const [rcon, setRcon] = useState<RconInfo | null>(null);
  const [rconPort, setRconPort] = useState(25575);
  const [rconCopied, setRconCopied] = useState(false);
  const [preview, setPreview] = useState<{
    template: PropertyTemplate;
    changes: PropertyChange[];
  } | null>(null);
  useEffect(() => {
    let live = true;
    void run(`properties-${server.id}-${crypto.randomUUID()}`, async () => {
      const [p, r, c] = await Promise.all([
        call("get_server_properties", { id: server.id }),
        call("get_raw_properties", { id: server.id }),
        call("list_content", { id: server.id }),
      ]);
      if (live) {
        setValues(p);
        setRaw(r);
        setJars(c);
      }
    });
    return () => {
      live = false;
    };
  }, [server.id]);
  useEffect(() => {
    let live = true;
    void run(`launch-${server.id}-${crypto.randomUUID()}`, async () => {
      const info = await call("launch_command", { id: server.id });
      if (live) {
        setLaunch(info);
        setCopied(false);
      }
    });
    return () => {
      live = false;
    };
  }, [
    server.id,
    server.installed,
    server.memoryMinMb,
    server.memoryMaxMb,
    server.jvmPreset,
    server.jvmFlags,
    server.javaPath,
  ]);
  useEffect(() => {
    let live = true;
    void run(`rcon-${server.id}-${crypto.randomUUID()}`, async () => {
      const info = await call("rcon_info", { id: server.id });
      if (live) {
        setRcon(info);
        setRconPort(info.port);
      }
    });
    return () => {
      live = false;
    };
  }, [server.id, server.runtime.status]);
  const copyCommand = () => {
    if (!launch) return;
    void run("copyLaunchCommand", async () => {
      await navigator.clipboard.writeText(launch.command);
      setCopied(true);
      window.setTimeout(() => setCopied(false), 1600);
    });
  };
  const change = (key: string, value: string) => {
    setValues((v) => ({ ...v, [key]: value }));
    setSaved(false);
  };
  const editable = idle(server.runtime.status);
  return (
    <div className="settings-stack">
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Settings2 size={18} />
              {t("serverSettings.world.title")}
            </h2>
            <p>{t("serverSettings.world.desc")}</p>
          </div>
          <Button
            disabled={!editable || !values}
            variant="primary"
            busy={busy.has("saveProperties")}
            onClick={() =>
              void run("saveProperties", async () => {
                await call("update_server_properties", {
                  id: server.id,
                  ...(advanced ? { raw } : { properties: values! }),
                });
                const [p, r] = await Promise.all([
                  call("get_server_properties", { id: server.id }),
                  call("get_raw_properties", { id: server.id }),
                ]);
                setValues(p);
                setRaw(r);
                setSaved(true);
                await onSaved();
              })
            }
          >
            <Save size={16} />
            {t("serverSettings.save")}
          </Button>
        </div>
        {saved && (
          <div className="info-strip success-text">
            {t("serverSettings.saved")}
          </div>
        )}
        <div className="segmented">
          <button
            className={!advanced ? "selected" : ""}
            onClick={() => setAdvanced(false)}
          >
            {t("serverSettings.common")}
          </button>
          <button
            className={advanced ? "selected" : ""}
            onClick={() => {
              setAdvanced(true);
            }}
          >
            {t("serverSettings.advanced")}
          </button>
        </div>
        {!advanced && values && (
          <div className="configuration-presets">
            <strong>{t("serverSettings.profile.title")}</strong>
            <div className="row">
              {(["small", "balanced", "performance"] as const).map(
                (profile, i) => (
                  <Button
                    key={profile}
                    disabled={!editable}
                    onClick={() => {
                      setValues((v) => ({
                        ...v,
                        "view-distance": ["8", "10", "6"][i],
                        "simulation-distance": ["4", "6", "4"][i],
                        "max-players": ["10", "20", "10"][i],
                      }));
                      setSaved(false);
                    }}
                  >
                    {t(`serverSettings.profile.${profile}`)}
                  </Button>
                ),
              )}
            </div>
            <p className="small muted">{t("serverSettings.profile.hint")}</p>
          </div>
        )}
        {!values ? (
          <p className="muted">{t("serverSettings.loading")}</p>
        ) : advanced ? (
          <>
            <p className="small muted">{t("serverSettings.rawHint")}</p>
            <textarea
              className="raw-editor"
              aria-label={t("serverSettings.rawAria")}
              spellCheck={false}
              disabled={!editable}
              value={raw}
              onChange={(e) => {
                setRaw(e.target.value);
                setSaved(false);
              }}
            />
          </>
        ) : (
          <fieldset disabled={!editable} className="form-grid">
            <Field label={t("serverSettings.motd")}>
              <input
                value={values.motd || ""}
                onChange={(e) => change("motd", e.target.value)}
              />
            </Field>
            {numeric.map(([k, label, lo, hi]) => (
              <Field label={t(label)} key={k}>
                <input
                  type="number"
                  min={lo}
                  max={hi}
                  value={values[k] || ""}
                  onChange={(e) => change(k, e.target.value)}
                />
              </Field>
            ))}
            {(
              [
                [
                  "gamemode",
                  "wizard.gameMode",
                  ["survival", "creative", "adventure", "spectator"],
                ],
                [
                  "difficulty",
                  "wizard.difficulty",
                  ["peaceful", "easy", "normal", "hard"],
                ],
              ] as const
            ).map(([k, label, opts]) => (
              <Field label={t(label)} key={k}>
                <select
                  value={values[k] || ""}
                  onChange={(e) => change(k, e.target.value)}
                >
                  {opts.map((v) => (
                    <option key={v} value={v}>
                      {t(`value.${v}` as MessageKey)}
                    </option>
                  ))}
                </select>
              </Field>
            ))}
            {(
              [
                ["pvp", "serverSettings.pvp"],
                ["online-mode", "serverSettings.onlineMode"],
                ["allow-flight", "serverSettings.allowFlight"],
                ["white-list", "serverSettings.whitelist"],
              ] as const
            ).map(([k, label]) => (
              <label key={k} className="check-row">
                <input
                  type="checkbox"
                  checked={values[k] === "true"}
                  onChange={(e) => change(k, String(e.target.checked))}
                />
                <span>{t(label)}</span>
              </label>
            ))}
          </fieldset>
        )}
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Radio size={18} />
              {t("serverSettings.rcon.title")}
            </h2>
            <p>{t("serverSettings.rcon.desc")}</p>
          </div>
          {rcon?.enabled ? (
            <Button
              disabled={!editable}
              busy={busy.has("rcon")}
              onClick={() =>
                void run("rcon", async () => {
                  const info = await call("rcon_disable", {
                    id: server.id,
                  });
                  setRcon(info);
                  setRconPort(info.port);
                  setRconCopied(false);
                })
              }
            >
              {t("serverSettings.rcon.disable")}
            </Button>
          ) : (
            <Button
              variant="primary"
              disabled={!editable}
              busy={busy.has("rcon")}
              onClick={() =>
                void run("rcon", async () => {
                  const info = await call("rcon_enable", {
                    id: server.id,
                    port: rconPort,
                  });
                  setRcon(info);
                  setRconPort(info.port);
                  setRconCopied(false);
                })
              }
            >
              {t("serverSettings.rcon.enable")}
            </Button>
          )}
        </div>
        {!rcon ? (
          <p className="muted">{t("serverSettings.loading")}</p>
        ) : rcon.enabled ? (
          <>
            <div className="info-strip success-text">
              {t("serverSettings.rcon.enabled", { port: rcon.port })}
            </div>
            {rcon.password ? (
              <Field label={t("serverSettings.rcon.password")}>
                <div className="row">
                  <input readOnly value={rcon.password} />
                  <Button
                    busy={busy.has("rcon-copy")}
                    onClick={() =>
                      void run("rcon-copy", async () => {
                        await navigator.clipboard.writeText(
                          rcon.password || "",
                        );
                        setRconCopied(true);
                        window.setTimeout(() => setRconCopied(false), 1600);
                      })
                    }
                  >
                    {rconCopied ? <Check size={16} /> : <Copy size={16} />}
                    {t(
                      rconCopied
                        ? "serverSettings.rcon.copied"
                        : "serverSettings.rcon.copy",
                    )}
                  </Button>
                </div>
              </Field>
            ) : (
              <p className="muted small">
                {t("serverSettings.rcon.passwordMissing")}
              </p>
            )}
          </>
        ) : (
          <>
            <Field label={t("serverSettings.rcon.port")}>
              <input
                type="number"
                min={1024}
                max={65535}
                disabled={!editable}
                value={rconPort}
                onChange={(e) => setRconPort(+e.target.value)}
              />
            </Field>
            <p className="small muted">{t("serverSettings.rcon.hint")}</p>
          </>
        )}
        <p className="small muted">{t("serverSettings.rcon.warning")}</p>
        <p className="small muted">{t("serverSettings.rcon.restart")}</p>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Layers size={18} />
              {t("serverSettings.template.title")}
            </h2>
            <p>{t("serverSettings.template.desc")}</p>
          </div>
        </div>
        <div className="row template-row">
          {templates.map((name) => (
            <Button
              key={name}
              disabled={!editable}
              busy={busy.has(`template-${name}`)}
              onClick={() =>
                void run(`template-${name}`, async () => {
                  const changes = await call("preview_properties_template", {
                    id: server.id,
                    template: name,
                  });
                  setPreview({ template: name, changes });
                })
              }
            >
              {t(`serverSettings.template.${name}` as MessageKey)}
            </Button>
          ))}
        </div>
        <p className="small muted">{t("serverSettings.template.hint")}</p>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>{t("serverSettings.runtime.title")}</h2>
            <p>
              {t("serverSettings.runtime.desc", {
                java:
                  server.requiredJava || t("serverSettings.runtime.compatible"),
              })}
            </p>
          </div>
          <Button
            disabled={!editable}
            busy={busy.has("resources")}
            onClick={() =>
              void run("resources", async () => {
                await call("update_resources", {
                  id: server.id,
                  min,
                  max,
                  javaPath: java,
                });
                await onSaved();
              })
            }
          >
            {t("serverSettings.runtime.save")}
          </Button>
        </div>
        <fieldset className="form-grid" disabled={!editable}>
          <Field label={t("serverSettings.runtime.min")}>
            <input
              type="number"
              step={256}
              min={256}
              value={min}
              onChange={(e) => setMin(+e.target.value)}
            />
          </Field>
          <Field label={t("serverSettings.runtime.max")}>
            <input
              type="number"
              step={256}
              min={256}
              value={max}
              onChange={(e) => setMax(+e.target.value)}
            />
          </Field>
          <Field label={t("serverSettings.runtime.java")}>
            <input value={java} onChange={(e) => setJava(e.target.value)} />
          </Field>
        </fieldset>
        <Button
          disabled={!editable}
          busy={busy.has("memory-suggestion")}
          onClick={() =>
            void run("memory-suggestion", async () => {
              const info = await call("system_info");
              const ceiling =
                Math.floor(
                  Math.min(
                    info.availableMb - 512,
                    info.totalMb * 0.8,
                    info.totalMb - 1024,
                  ) / 256,
                ) * 256;
              const suggested = Math.max(
                256,
                Math.min(info.recommendedMb, ceiling),
              );
              setMax(suggested);
              setMin(Math.min(1024, suggested));
            })
          }
        >
          {t("serverSettings.memory.recommend")}
        </Button>
        <p className="small muted">{t("serverSettings.memory.note")}</p>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Cpu size={18} />
              {t("serverSettings.jvm.title")}
            </h2>
            <p>{t("serverSettings.jvm.desc")}</p>
          </div>
          <Button
            disabled={!editable}
            busy={busy.has("jvm")}
            onClick={() =>
              void run("jvm", async () => {
                await call("update_jvm", {
                  id: server.id,
                  preset,
                  flags:
                    preset === "custom"
                      ? flags
                          .split("\n")
                          .map((f) => f.trim())
                          .filter(Boolean)
                      : [],
                });
                setJvmSaved(true);
                await onSaved();
              })
            }
          >
            <Save size={16} />
            {t("serverSettings.jvm.save")}
          </Button>
        </div>
        {jvmSaved && (
          <div className="info-strip success-text">
            {t("serverSettings.saved")}
          </div>
        )}
        <div className="segmented jvm-presets">
          {(["balanced", "aikar", "paper", "custom"] as const).map((p) => (
            <button
              key={p}
              disabled={!editable || busy.has("jvm")}
              className={preset === p ? "selected" : ""}
              onClick={() => {
                setPreset(p);
                setJvmSaved(false);
              }}
            >
              {t(`serverSettings.jvm.${p}` as MessageKey)}
            </button>
          ))}
        </div>
        <p className="small muted">
          {t(`serverSettings.jvm.${preset}Desc` as MessageKey)}
        </p>
        {preset === "custom" && (
          <Field label={t("serverSettings.jvm.customLabel")}>
            <textarea
              className="raw-editor jvm-flags"
              spellCheck={false}
              disabled={!editable}
              placeholder="-XX:+UseG1GC"
              value={flags}
              onChange={(e) => {
                setFlags(e.target.value);
                setJvmSaved(false);
              }}
            />
          </Field>
        )}
        <p className="small muted jvm-note">
          {t("serverSettings.jvm.customHint")}{" "}
          {t("serverSettings.jvm.memoryManaged")}
        </p>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Terminal size={18} />
              {t("serverSettings.launch.title")}
            </h2>
            <p>{t("serverSettings.launch.desc")}</p>
          </div>
          <Button
            disabled={!launch}
            busy={busy.has("copyLaunchCommand")}
            onClick={copyCommand}
          >
            {copied ? <Check size={16} /> : <Copy size={16} />}
            {t(
              copied
                ? "serverSettings.launch.copied"
                : "serverSettings.launch.copy",
            )}
          </Button>
        </div>
        {!launch ? (
          <p className="muted small">
            {t("serverSettings.launch.unavailable")}
          </p>
        ) : (
          <>
            {!launch.jarPresent && (
              <div className="info-strip">
                {t("serverSettings.launch.jarMissing")}
              </div>
            )}
            <pre className="launch-command">
              <code>{launch.command}</code>
            </pre>
            <dl className="launch-details">
              <div>
                <dt>{t("serverSettings.launch.java")}</dt>
                <dd>{launch.javaPath}</dd>
              </div>
              <div>
                <dt>{t("serverSettings.launch.dir")}</dt>
                <dd>{launch.workingDirectory}</dd>
              </div>
              <div>
                <dt>{t("serverSettings.launch.args")}</dt>
                <dd>{launch.args.join(" ")}</dd>
              </div>
            </dl>
          </>
        )}
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <FolderOpen size={18} />
              {t("serverSettings.files.title")}
            </h2>
            <p>{t("serverSettings.files.desc")}</p>
          </div>
          <div className="row">
            <Button
              onClick={() =>
                void run("openServerFolder", () =>
                  call("open_folder", { id: server.id, kind: "server" }),
                )
              }
            >
              <FolderOpen size={16} />
              {t("serverSettings.files.root")}
            </Button>
            <Button
              onClick={() =>
                void run("openServerLogs", () =>
                  call("open_folder", { id: server.id, kind: "logs" }),
                )
              }
            >
              {t("serverSettings.files.logs")}
            </Button>
            <Button
              onClick={() =>
                void run("openServerBackups", () =>
                  call("open_folder", { id: server.id, kind: "backups" }),
                )
              }
            >
              {t("serverSettings.files.backups")}
            </Button>
          </div>
        </div>
      </section>
      {server.serverType !== "vanilla" && (
        <section className="panel">
          <div className="section-title">
            <div>
              <h2>
                {server.serverType === "paper"
                  ? t("serverSettings.plugins")
                  : t("serverSettings.mods")}
              </h2>
              <p>{t("serverSettings.contentDesc")}</p>
            </div>
            <Button
              onClick={() =>
                void run("folder", () =>
                  call("open_folder", {
                    id: server.id,
                    kind: server.serverType === "paper" ? "plugins" : "mods",
                  }),
                )
              }
            >
              <FolderOpen size={16} />
              {t("common.openFolder")}
            </Button>
          </div>
          {jars.length ? (
            <ul className="jar-list">
              {jars.map((j) => (
                <li key={j}>{j}</li>
              ))}
            </ul>
          ) : (
            <p className="muted small">{t("serverSettings.noJars")}</p>
          )}
        </section>
      )}
      {preview && (
        <Modal
          title={t("serverSettings.template.previewTitle", {
            name: t(
              `serverSettings.template.${preview.template}` as MessageKey,
            ),
          })}
          onClose={() => setPreview(null)}
        >
          <p className="small muted">{t("serverSettings.template.previewHint")}</p>
          {preview.changes.length ? (
            <ul className="diff-list">
              {preview.changes.map((change) => (
                <li key={change.key} className="diff-row">
                  <code>{change.key}</code>
                  <div className="diff-values">
                    <span className="diff-before">
                      {change.before ?? t("serverSettings.template.unset")}
                    </span>
                    <ArrowRight size={14} />
                    <span className="diff-after">{change.after}</span>
                  </div>
                </li>
              ))}
            </ul>
          ) : (
            <p className="muted">{t("serverSettings.template.noChanges")}</p>
          )}
          <div className="modal-actions">
            <Button onClick={() => setPreview(null)}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              disabled={!preview.changes.length}
              busy={busy.has("applyTemplate")}
              onClick={() => {
                const template = preview.template;
                void run("applyTemplate", async () => {
                  await call("apply_properties_template", {
                    id: server.id,
                    template,
                  });
                  const [p, r] = await Promise.all([
                    call("get_server_properties", { id: server.id }),
                    call("get_raw_properties", { id: server.id }),
                  ]);
                  setValues(p);
                  setRaw(r);
                  setPreview(null);
                  setSaved(true);
                  await onSaved();
                });
              }}
            >
              {t("serverSettings.template.apply")}
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}
