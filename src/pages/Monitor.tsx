import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Activity,
  BellRing,
  Cpu,
  Eraser,
  Gauge,
  HeartPulse,
  MemoryStick,
  RefreshCw,
  Timer,
  TriangleAlert,
  Users,
} from "lucide-react";
import { Button, Field, date } from "../components/ui";
import { call, desktop } from "../services/api";
import { useI18n, type MessageKey } from "../i18n";
import { active } from "../types";
import type {
  AlertRules,
  MetricsSample,
  ServerAlert,
  ServerMetrics,
  ServerView,
} from "../types";
import type { TaskProps } from "../utils/task";

const MAX_POINTS = 60;
const VIEW = { width: 640, height: 180, pad: 14 };
const GRID = [25, 50, 75];

function points(history: MetricsSample[], key: "cpuPercent" | "memoryPercent") {
  const inner = VIEW.width - VIEW.pad * 2;
  const height = VIEW.height - VIEW.pad * 2;
  return history
    .map((sample, index) => {
      const x = VIEW.pad + (index / (history.length - 1)) * inner;
      const value = Math.min(100, Math.max(0, sample[key]));
      const y = VIEW.height - VIEW.pad - (value / 100) * height;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .join(" ");
}

export function Monitor({
  server,
  run,
  busy,
}: TaskProps & { server: ServerView }) {
  const { t } = useI18n();
  const [metrics, setMetrics] = useState<ServerMetrics | null>(null);
  const [history, setHistory] = useState<MetricsSample[]>([]);
  const [alerts, setAlerts] = useState<ServerAlert[]>([]);
  const [rules, setRules] = useState<AlertRules | null>(null);
  const [draft, setDraft] = useState<AlertRules | null>(null);
  const [message, setMessage] = useState("");
  const [recovery, setRecovery] = useState(server.autoRestart);
  const [recoveryMessage, setRecoveryMessage] = useState("");
  const key = `monitor-${server.id}`;
  useEffect(() => setRecovery(server.autoRestart), [server.autoRestart]);
  const load = async () => {
    const [nextMetrics, nextHistory, nextAlerts, nextRules] = await Promise.all([
      call("monitor_metrics", { id: server.id }),
      call("monitor_history", { id: server.id }),
      call("monitor_alerts", { id: server.id }),
      call("monitor_rules", { id: server.id }),
    ]);
    setMetrics(nextMetrics);
    setHistory(nextHistory);
    setAlerts(nextAlerts);
    setRules(nextRules);
    setDraft(nextRules);
  };
  useEffect(() => {
    void run(`${key}-${crypto.randomUUID()}`, load);
  }, [server.id]);
  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    const metricsEvent = listen<ServerMetrics>("server-metrics", (event) => {
      const payload = event.payload;
      if (!alive || payload.id !== server.id) return;
      setMetrics(payload);
      setHistory((previous) =>
        [
          ...previous,
          {
            at: payload.sampledAt,
            cpuPercent: payload.cpuPercent,
            memoryPercent: payload.memoryPercent,
            players: payload.players,
          },
        ].slice(-MAX_POINTS),
      );
    });
    const alertEvent = listen<ServerAlert>("server-alert", (event) => {
      const payload = event.payload;
      if (!alive || payload.serverId !== server.id) return;
      setAlerts((previous) => [payload, ...previous].slice(0, 50));
    });
    return () => {
      alive = false;
      void metricsEvent.then((off) => off());
      void alertEvent.then((off) => off());
    };
  }, [server.id]);
  useEffect(() => {
    if (!active(server.runtime.status)) setMetrics(null);
  }, [server.runtime.status]);
  const uptime = (secs: number) => {
    const hours = Math.floor(secs / 3600);
    const minutes = Math.floor((secs % 3600) / 60);
    return hours > 0
      ? t("monitor.uptimeValue", { h: hours, m: minutes })
      : t("monitor.uptimeShort", { m: minutes });
  };
  const alertValue = (alert: ServerAlert) => {
    const unit = alert.kind === "lag" ? "" : "%";
    return t("monitor.alertValue", {
      value: `${alert.value.toFixed(alert.kind === "lag" ? 0 : 1)}${unit}`,
      threshold: `${alert.threshold.toFixed(alert.kind === "lag" ? 0 : 1)}${unit}`,
    });
  };
  const cards = metrics
    ? [
        {
          id: "cpu",
          icon: Cpu,
          label: t("monitor.cpu"),
          value: `${metrics.cpuPercent.toFixed(1)}%`,
          hint: t("monitor.cpuHint"),
        },
        {
          id: "memory",
          icon: MemoryStick,
          label: t("monitor.memory"),
          value: `${metrics.memoryPercent.toFixed(1)}%`,
          hint: t("monitor.memoryHint", {
            used: metrics.memoryMb,
            limit: metrics.memoryLimitMb,
          }),
        },
        {
          id: "players",
          icon: Users,
          label: t("monitor.players"),
          value: `${metrics.players} / ${metrics.maxPlayers}`,
          hint: t("monitor.playersHint", { max: metrics.maxPlayers }),
        },
        {
          id: "uptime",
          icon: Timer,
          label: t("monitor.uptime"),
          value: uptime(metrics.uptimeSecs),
          hint: date(metrics.sampledAt),
        },
        {
          id: "lag",
          icon: Gauge,
          label: t("monitor.lag"),
          value: String(metrics.lagEvents),
          hint: t("monitor.lagHint"),
          warn: metrics.lagEvents >= (rules?.lagEvents ?? 3),
        },
      ]
    : [];
  const cpuLine = points(history, "cpuPercent");
  const memoryLine = points(history, "memoryPercent");
  const exitLabel = (code: number) =>
    code < 0 ? `0x${(code >>> 0).toString(16).toUpperCase()}` : String(code);
  const toggleRecovery = () =>
    run(`${key}-recovery`, async () => {
      const updated = await call("update_auto_restart", {
        id: server.id,
        enabled: !recovery,
      });
      setRecovery(updated.autoRestart);
      setRecoveryMessage(t("recovery.saved"));
    });
  return (
    <div className="monitor-stack">
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Activity size={18} />
              {t("monitor.title")}
            </h2>
            <p>{t("monitor.desc")}</p>
          </div>
          <div className="monitor-actions">
            {metrics && (
              <span className="live-pill">
                <span className="live-dot" />
                {t("monitor.live")}
              </span>
            )}
            <Button busy={busy.has(key)} onClick={() => void run(key, load)}>
              <RefreshCw size={15} />
              {t("monitor.refresh")}
            </Button>
          </div>
        </div>
        {metrics ? (
          <div className="metric-grid">
            {cards.map((card) => {
              const Icon = card.icon;
              return (
                <article
                  className={`metric-card ${card.warn ? "warn" : ""}`}
                  key={card.id}
                >
                  <span className="metric-icon">
                    <Icon size={16} />
                  </span>
                  <small>{card.label}</small>
                  <strong>{card.value}</strong>
                  <em>{card.hint}</em>
                </article>
              );
            })}
          </div>
        ) : (
          <p className="mini-empty">
            {busy.has(key) ? t("common.loading") : t("monitor.noData")}
          </p>
        )}
        <div className="chart-card">
          <div className="chart-head">
            <strong>{t("monitor.chartTitle")}</strong>
            <div className="chart-legend">
              <span className="cpu">
                <i />
                {t("monitor.chartCpu")}
              </span>
              <span className="memory">
                <i />
                {t("monitor.chartMemory")}
              </span>
            </div>
          </div>
          {history.length > 1 ? (
            <svg
              className="spark"
              viewBox={`0 0 ${VIEW.width} ${VIEW.height}`}
              role="img"
              aria-label={t("monitor.chartTitle")}
            >
              {GRID.map((value) => {
                const y =
                  VIEW.height -
                  VIEW.pad -
                  (value / 100) * (VIEW.height - VIEW.pad * 2);
                return (
                  <line
                    key={value}
                    className="spark-grid"
                    x1={VIEW.pad}
                    x2={VIEW.width - VIEW.pad}
                    y1={y}
                    y2={y}
                  />
                );
              })}
              <polyline className="spark-line memory" points={memoryLine} />
              <polyline className="spark-line cpu" points={cpuLine} />
            </svg>
          ) : (
            <p className="mini-empty">{t("monitor.chartEmpty")}</p>
          )}
        </div>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <BellRing size={18} />
              {t("monitor.rulesTitle")}
            </h2>
            <p>{t("monitor.rulesDesc")}</p>
          </div>
        </div>
        {draft ? (
          <>
            <div className="rule-grid">
              <div className="task-toggle">
                <button
                  type="button"
                  className={`switch ${draft.enabled ? "on" : ""}`}
                  role="switch"
                  aria-checked={draft.enabled}
                  aria-label={t("monitor.rulesEnabled")}
                  onClick={() =>
                    setDraft({ ...draft, enabled: !draft.enabled })
                  }
                />
                <span>{t("monitor.rulesEnabled")}</span>
              </div>
              <Field label={t("monitor.cpuThreshold")}>
                <input
                  type="number"
                  min={10}
                  max={100}
                  value={draft.cpuPercent}
                  onChange={(e) =>
                    setDraft({ ...draft, cpuPercent: Number(e.target.value) })
                  }
                />
              </Field>
              <Field label={t("monitor.memoryThreshold")}>
                <input
                  type="number"
                  min={10}
                  max={100}
                  value={draft.memoryPercent}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      memoryPercent: Number(e.target.value),
                    })
                  }
                />
              </Field>
              <Field
                label={t("monitor.lagThreshold")}
                hint={t("monitor.lagThresholdHint")}
              >
                <input
                  type="number"
                  min={1}
                  max={100}
                  value={draft.lagEvents}
                  onChange={(e) =>
                    setDraft({ ...draft, lagEvents: Number(e.target.value) })
                  }
                />
              </Field>
            </div>
            {message && (
              <p className="success-text" role="status">
                {message}
              </p>
            )}
            <div className="row">
              <Button
                variant="primary"
                busy={busy.has(`${key}-save`)}
                disabled={!rules || busy.has(`${key}-save`)}
                onClick={() =>
                  void run(`${key}-save`, async () => {
                    const saved = await call("save_monitor_rules", {
                      id: server.id,
                      rules: draft,
                    });
                    setRules(saved);
                    setDraft(saved);
                    setMessage(t("monitor.rulesSaved"));
                  })
                }
              >
                {t("monitor.saveRules")}
              </Button>
            </div>
          </>
        ) : (
          <p className="mini-empty">{t("common.loading")}</p>
        )}
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <HeartPulse size={18} />
              {t("recovery.title")}
            </h2>
            <p>{t("recovery.desc")}</p>
          </div>
        </div>
        <div className="rule-grid">
          <div className="task-toggle">
            <button
              type="button"
              className={`switch ${recovery ? "on" : ""}`}
              role="switch"
              aria-checked={recovery}
              aria-label={t("recovery.enabled")}
              disabled={busy.has(`${key}-recovery`)}
              onClick={() => void toggleRecovery()}
            />
            <span>{t("recovery.enabled")}</span>
          </div>
          <div>
            <small className="muted">{t("recovery.lastExit")}</small>
            {server.runtime.exitCode === null ? (
              <p className="muted">{t("recovery.noExit")}</p>
            ) : (
              <p>
                <strong>
                  {t("recovery.exitCode", {
                    code: exitLabel(server.runtime.exitCode),
                  })}
                </strong>
                {server.runtime.exitReason && (
                  <>
                    <br />
                    <small className="muted">
                      {server.runtime.exitReason}
                    </small>
                  </>
                )}
              </p>
            )}
          </div>
        </div>
        {recoveryMessage && (
          <p className="success-text" role="status">
            {recoveryMessage}
          </p>
        )}
        <p className="muted">{t("recovery.hint")}</p>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <TriangleAlert size={18} />
              {t("monitor.alertsTitle")}
            </h2>
            <p>{t("monitor.alertsDesc")}</p>
          </div>
          <Button
            disabled={!alerts.length}
            onClick={() =>
              void run(`${key}-clear`, async () => {
                await call("clear_monitor_alerts", { id: server.id });
                setAlerts([]);
              })
            }
          >
            <Eraser size={15} />
            {t("monitor.clearAlerts")}
          </Button>
        </div>
        {alerts.length ? (
          <div className="alert-list">
            {alerts.map((alert) => (
              <article className={`alert-item ${alert.kind}`} key={alert.id}>
                <span className="alert-icon">
                  <TriangleAlert size={15} />
                </span>
                <div>
                  <strong>
                    {t(`monitor.alert.${alert.kind}` as MessageKey)}
                  </strong>
                  <small>{alertValue(alert)}</small>
                </div>
                <time>{date(alert.at)}</time>
              </article>
            ))}
          </div>
        ) : (
          <div className="mini-empty">
            <BellRing size={36} />
            <h3>{t("monitor.noAlertsTitle")}</h3>
            <p>{t("monitor.noAlertsDesc")}</p>
          </div>
        )}
      </section>
    </div>
  );
}
