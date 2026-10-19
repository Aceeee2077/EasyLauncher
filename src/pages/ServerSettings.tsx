import { useEffect, useState } from "react";
import {
  Save,
  FolderOpen,
  Settings2,
  Terminal,
  Copy,
  Check,
} from "lucide-react";
import { Button, Field } from "../components/ui";
import { call } from "../services/api";
import { idle, type LaunchInfo, type ServerView } from "../types";
import type { TaskProps } from "../utils/task";
import { useI18n, type MessageKey } from "../i18n";
const numeric = [
  ["server-port", "serverSettings.port", 1, 65535],
  ["max-players", "serverSettings.maxPlayers", 1, 1000],
  ["spawn-protection", "serverSettings.spawnProtection", 0, 10000],
  ["view-distance", "serverSettings.viewDistance", 2, 32],
  ["simulation-distance", "serverSettings.simulationDistance", 2, 32],
] as const satisfies readonly (readonly [string, MessageKey, number, number])[];
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
  const [launch, setLaunch] = useState<LaunchInfo | null>(null);
  const [copied, setCopied] = useState(false);
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
    server.javaPath,
  ]);
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
          <p className="muted small">{t("serverSettings.launch.unavailable")}</p>
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
    </div>
  );
}
