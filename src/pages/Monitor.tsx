import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import QRCode from "qrcode";
import {
  Activity,
  BellRing,
  Copy,
  Cpu,
  Eraser,
  Flame,
  Gauge,
  HeartPulse,
  MemoryStick,
  RefreshCw,
  Smartphone,
  Timer,
  TriangleAlert,
  Users,
} from "lucide-react";
import { Button, Field, date } from "../components/ui";
import { call, desktop } from "../services/api";
import { useI18n, type MessageKey } from "../i18n";
import { errorMessageKey } from "../i18n/messages";
import { active } from "../types";
import type {
  AlertRules,
  HeapInfo,
  JfrTools,
  MetricsSample,
  PerfConfig,
  PerfReport,
  PerfSample,
  ServerAlert,
  ServerMetrics,
  ServerView,
  WebConsoleInfo,
} from "../types";
import type { TaskProps } from "../utils/task";

const MAX_POINTS = 60;
const PERF_POINTS = 180;
const PERF_TPS_MAX = 20;
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

function perfPoints(
  samples: PerfSample[],
  pick: (sample: PerfSample) => number | null,
  max: number,
) {
  const inner = VIEW.width - VIEW.pad * 2;
  const height = VIEW.height - VIEW.pad * 2;
  const usable = samples
    .map((sample, index) => {
      const value = pick(sample);
      if (value === null || !Number.isFinite(value)) return null;
      const x = VIEW.pad + (index / Math.max(1, samples.length - 1)) * inner;
      const y =
        VIEW.height -
        VIEW.pad -
        (Math.min(max, Math.max(0, value)) / max) * height;
      return `${x.toFixed(1)},${y.toFixed(1)}`;
    })
    .filter((point): point is string => point !== null);
  return { line: usable.join(" "), count: usable.length };
}

function TrendChart({
  title,
  line,
  count,
  max,
  tone,
  empty,
}: {
  title: string;
  line: string;
  count: number;
  max: number;
  tone: "tps" | "mspt" | "memory";
  empty: string;
}) {
  const height = VIEW.height - VIEW.pad * 2;
  return (
    <div className="chart-card perf-chart">
      <div className="chart-head">
        <strong>{title}</strong>
        <span className="muted">{max.toFixed(0)}</span>
      </div>
      {count > 1 ? (
        <svg
          className="spark"
          viewBox={`0 0 ${VIEW.width} ${VIEW.height}`}
          role="img"
          aria-label={title}
        >
          {[1 / 3, 2 / 3, 1].map((share) => {
            const y = VIEW.height - VIEW.pad - share * height;
            return (
              <line
                key={share}
                className="spark-grid"
                x1={VIEW.pad}
                x2={VIEW.width - VIEW.pad}
                y1={y}
                y2={y}
              />
            );
          })}
          <polyline className={`spark-line ${tone}`} points={line} />
        </svg>
      ) : (
        <p className="mini-empty">{empty}</p>
      )}
    </div>
  );
}

function fileSize(bytes: number) {
  return bytes >= 1024 * 1024
    ? `${(bytes / 1024 / 1024).toFixed(1)} MB`
    : `${Math.max(1, Math.round(bytes / 1024))} KB`;
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
  const [web, setWeb] = useState<WebConsoleInfo | null>(null);
  const [qr, setQr] = useState("");
  const [webMessage, setWebMessage] = useState("");
  const [perf, setPerf] = useState<PerfReport | null>(null);
  const [perfDraft, setPerfDraft] = useState<PerfConfig | null>(null);
  const [perfMessage, setPerfMessage] = useState("");
  const [jfrTools, setJfrTools] = useState<JfrTools | null>(null);
  const [jfrDuration, setJfrDuration] = useState(60);
  const [heap, setHeap] = useState<HeapInfo | null>(null);
  const [jfrMessage, setJfrMessage] = useState("");
  const key = `monitor-${server.id}`;
  useEffect(() => setRecovery(server.autoRestart), [server.autoRestart]);
  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    call("web_console_info")
      .then((info) => {
        if (alive) setWeb(info);
      })
      .catch(() => undefined);
    return () => {
      alive = false;
    };
  }, []);
  useEffect(() => {
    if (!web) {
      setQr("");
      return;
    }
    let alive = true;
    QRCode.toDataURL(web.url, {
      width: 240,
      margin: 1,
      color: { dark: "#0d1a12", light: "#ffffff" },
    })
      .then((data) => {
        if (alive) setQr(data);
      })
      .catch(() => {
        if (alive) setQr("");
      });
    return () => {
      alive = false;
    };
  }, [web]);
  const load = async () => {
    const [nextMetrics, nextHistory, nextAlerts, nextRules, nextPerf, nextTools] =
      await Promise.all([
        call("monitor_metrics", { id: server.id }),
        call("monitor_history", { id: server.id }),
        call("monitor_alerts", { id: server.id }),
        call("monitor_rules", { id: server.id }),
        call("perf_report", { id: server.id }),
        call("perf_jfr_tools", { id: server.id }),
      ]);
    setMetrics(nextMetrics);
    setHistory(nextHistory);
    setAlerts(nextAlerts);
    setRules(nextRules);
    setDraft(nextRules);
    setPerf(nextPerf);
    setPerfDraft({
      enabled: nextPerf.enabled,
      intervalSecs: nextPerf.intervalSecs,
    });
    setJfrTools(nextTools);
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
    const perfEvent = listen<PerfSample>("server-perf", (event) => {
      const payload = event.payload;
      if (!alive || payload.id !== server.id) return;
      setPerf((previous) =>
        previous
          ? {
              ...previous,
              latest: payload,
              lastError: null,
              samples: [...previous.samples, payload].slice(-720),
            }
          : previous,
      );
    });
    return () => {
      alive = false;
      void metricsEvent.then((off) => off());
      void alertEvent.then((off) => off());
      void perfEvent.then((off) => off());
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
  const toggleWeb = () =>
    run(`${key}-web`, async () => {
      if (web) {
        await call("stop_web_console");
        setWeb(null);
        setWebMessage(t("remote.stopped"));
      } else {
        const info = await call("start_web_console");
        setWeb(info);
        setWebMessage(t("remote.started"));
      }
    });
  const copyWeb = () =>
    run(`${key}-web-copy`, async () => {
      if (!web) return;
      await navigator.clipboard.writeText(web.url);
      setWebMessage(t("remote.copied"));
    });
  const running = server.runtime.status === "running";
  const latestPerf = perf?.latest ?? null;
  const trends = perf?.samples.slice(-PERF_POINTS) ?? [];
  const tpsTrend = perfPoints(trends, (sample) => sample.tps1m, PERF_TPS_MAX);
  const msptMax = Math.max(50, ...trends.map((sample) => sample.mspt ?? 0));
  const msptTrend = perfPoints(trends, (sample) => sample.mspt, msptMax);
  const memoryMax = Math.max(
    server.memoryMaxMb,
    ...trends.map((sample) => sample.memoryMb ?? 0),
    1,
  );
  const memoryTrend = perfPoints(
    trends,
    (sample) => sample.memoryMb,
    memoryMax,
  );
  const perfError = perf?.lastError
    ? (() => {
        const errorKey = errorMessageKey(perf.lastError!.code);
        return errorKey ? t(errorKey) : perf.lastError!.message;
      })()
    : "";
  const mb = (value: number | null) =>
    value === null ? "—" : t("perf.mb", { value: value.toFixed(1) });
  const savePerf = (next: PerfConfig) => {
    void run(`${key}-perf-save`, async () => {
      const saved = await call("perf_save_config", {
        id: server.id,
        config: next,
      });
      setPerf((previous) =>
        previous
          ? {
              ...previous,
              enabled: saved.enabled,
              intervalSecs: saved.intervalSecs,
            }
          : previous,
      );
      setPerfDraft(saved);
      setPerfMessage(t("perf.saved"));
    });
  };
  const samplePerf = () =>
    run(`${key}-perf-sample`, async () => {
      await call("perf_sample_now", { id: server.id });
      setPerf(await call("perf_report", { id: server.id }));
      setPerfMessage("");
    });
  const startJfr = () =>
    run(`${key}-jfr-start`, async () => {
      await call("perf_jfr_start", {
        id: server.id,
        durationSecs: jfrDuration,
        settings: "profile",
      });
      setJfrMessage(t("perf.jfrStarted", { seconds: jfrDuration }));
      setPerf(await call("perf_report", { id: server.id }));
    });
  const stopJfr = (name: string) =>
    run(`${key}-jfr-stop`, async () => {
      await call("perf_jfr_stop", { id: server.id, name });
      setJfrMessage(t("perf.jfrStopped"));
      setPerf(await call("perf_report", { id: server.id }));
    });
  const deleteProfile = (name: string) => {
    if (!window.confirm(t("perf.jfrDeleteConfirm"))) return;
    void run(`${key}-jfr-delete`, async () => {
      await call("perf_delete_profile", { id: server.id, name });
      setJfrMessage(t("perf.jfrDeleted"));
      setPerf(await call("perf_report", { id: server.id }));
    });
  };
  const readHeap = () =>
    run(`${key}-heap`, async () => {
      setHeap(await call("perf_heap_info", { id: server.id }));
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
              <Gauge size={18} />
              {t("perf.title")}
            </h2>
            <p>{t("perf.desc")}</p>
          </div>
          <div className="monitor-actions">
            {perf?.spark && <span className="live-pill">{t("perf.spark")}</span>}
            <span className="muted">
              {t("perf.samples", { count: perf?.samples.length ?? 0 })}
            </span>
            <Button
              busy={busy.has(`${key}-perf-sample`)}
              disabled={!running || busy.has(`${key}-perf-sample`)}
              onClick={() => void samplePerf()}
            >
              <RefreshCw size={15} />
              {t("perf.sampleNow")}
            </Button>
          </div>
        </div>
        <div className="rule-grid perf-settings">
          <div className="task-toggle">
            <button
              type="button"
              className={`switch ${perf?.enabled ? "on" : ""}`}
              role="switch"
              aria-checked={Boolean(perf?.enabled)}
              aria-label={t("perf.record")}
              disabled={!perf || busy.has(`${key}-perf-save`)}
              onClick={() =>
                savePerf({
                  enabled: !perf?.enabled,
                  intervalSecs:
                    perfDraft?.intervalSecs ?? perf?.intervalSecs ?? 10,
                })
              }
            />
            <span>{t("perf.record")}</span>
          </div>
          <Field label={t("perf.interval")} hint={t("perf.intervalHint")}>
            <input
              type="number"
              min={5}
              max={60}
              value={perfDraft?.intervalSecs ?? 10}
              onChange={(event) =>
                setPerfDraft((previous) =>
                  previous
                    ? {
                        ...previous,
                        intervalSecs: Number(event.target.value),
                      }
                    : previous,
                )
              }
            />
          </Field>
          <Button
            busy={busy.has(`${key}-perf-save`)}
            disabled={!perfDraft || busy.has(`${key}-perf-save`)}
            onClick={() =>
              savePerf({
                enabled: perf?.enabled ?? false,
                intervalSecs: perfDraft?.intervalSecs ?? 10,
              })
            }
          >
            {t("monitor.saveRules")}
          </Button>
        </div>
        {perfError && (
          <p className="warn-text" role="status">
            {t("perf.lastError")}
            {perfError}
          </p>
        )}
        {perf && !perf.rconReady && <p className="muted">{t("perf.needRcon")}</p>}
        {perfMessage && (
          <p className="success-text" role="status">
            {perfMessage}
          </p>
        )}
        {latestPerf ? (
          <div className="perf-stats">
            <div>
              <small>{t("perf.tps1m")}</small>
              <strong>
                {latestPerf.tps1m === null
                  ? "—"
                  : latestPerf.tps1m.toFixed(1)}
              </strong>
              <em>{t("perf.tpsHint")}</em>
            </div>
            <div>
              <small>{t("perf.tps5m")}</small>
              <strong>
                {latestPerf.tps5m === null
                  ? "—"
                  : latestPerf.tps5m.toFixed(1)}
              </strong>
            </div>
            <div>
              <small>{t("perf.tps15m")}</small>
              <strong>
                {latestPerf.tps15m === null
                  ? "—"
                  : latestPerf.tps15m.toFixed(1)}
              </strong>
            </div>
            <div>
              <small>{t("perf.mspt")}</small>
              <strong>
                {latestPerf.mspt === null
                  ? "—"
                  : latestPerf.mspt.toFixed(1)}
              </strong>
              <em>{t("perf.msptHint")}</em>
            </div>
          </div>
        ) : (
          <p className="mini-empty">
            {perf ? t("perf.noSamples") : t("common.loading")}
          </p>
        )}
        <div className="perf-charts">
          <TrendChart
            title={t("perf.chartTps")}
            line={tpsTrend.line}
            count={tpsTrend.count}
            max={PERF_TPS_MAX}
            tone="tps"
            empty={t("perf.noSamples")}
          />
          <TrendChart
            title={t("perf.chartMspt")}
            line={msptTrend.line}
            count={msptTrend.count}
            max={msptMax}
            tone="mspt"
            empty={t("perf.noSamples")}
          />
          <TrendChart
            title={t("perf.chartMemory")}
            line={memoryTrend.line}
            count={memoryTrend.count}
            max={memoryMax}
            tone="memory"
            empty={t("perf.noSamples")}
          />
        </div>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Flame size={18} />
              {t("perf.jfrTitle")}
            </h2>
            <p>{t("perf.jfrDesc")}</p>
          </div>
        </div>
        {jfrTools ? (
          <p className={`jfr-tools ${jfrTools.available ? "ready" : "blocked"}`}>
            {jfrTools.available
              ? t("perf.jfrReady", { path: jfrTools.jcmd ?? "" })
              : t(
                  jfrTools.reason === "jreOnly"
                    ? "perf.jfrJreOnly"
                    : "perf.jfrNoJava",
                )}
          </p>
        ) : (
          <p className="mini-empty">{t("common.loading")}</p>
        )}
        <div className="rule-grid">
          <Field label={t("perf.jfrDuration")}>
            <select
              value={jfrDuration}
              onChange={(event) => setJfrDuration(Number(event.target.value))}
            >
              <option value={30}>30s</option>
              <option value={60}>60s</option>
              <option value={120}>2 min</option>
              <option value={300}>5 min</option>
            </select>
          </Field>
          <Button
            variant="primary"
            busy={busy.has(`${key}-jfr-start`)}
            disabled={
              !jfrTools?.available ||
              !running ||
              busy.has(`${key}-jfr-start`)
            }
            onClick={() => void startJfr()}
          >
            <Flame size={15} />
            {t("perf.jfrStart")}
          </Button>
          <Button
            busy={busy.has(`${key}-heap`)}
            disabled={!jfrTools?.available || !running}
            onClick={() => void readHeap()}
          >
            {t("perf.heapRead")}
          </Button>
        </div>
        {!running && <p className="muted">{t("perf.notRunning")}</p>}
        {jfrMessage && (
          <p className="success-text" role="status">
            {jfrMessage}
          </p>
        )}
        {perf && perf.recordings.length > 0 && (
          <>
            <h3 className="perf-subtitle">{t("perf.jfrActive")}</h3>
            <div className="jfr-list">
              {perf.recordings.map((record) => (
                <article className="jfr-item" key={record.name}>
                  <div>
                    <strong>{record.file}</strong>
                    <small>
                      {date(record.startedAt)} · {record.durationSecs}s
                    </small>
                  </div>
                  <Button
                    busy={busy.has(`${key}-jfr-stop`)}
                    disabled={!running}
                    onClick={() => void stopJfr(record.name)}
                  >
                    {t("perf.jfrStop")}
                  </Button>
                </article>
              ))}
            </div>
          </>
        )}
        <h3 className="perf-subtitle">{t("perf.jfrProfiles")}</h3>
        {perf && perf.profiles.length > 0 ? (
          <div className="jfr-list">
            {perf.profiles.map((profile) => (
              <article className="jfr-item" key={profile.name}>
                <div>
                  <strong>{profile.name}</strong>
                  <small>
                    {fileSize(profile.sizeBytes)}
                    {profile.modifiedAt ? ` · ${date(profile.modifiedAt)}` : ""}
                  </small>
                </div>
                <Button
                  busy={busy.has(`${key}-jfr-delete`)}
                  onClick={() => deleteProfile(profile.name)}
                >
                  {t("perf.jfrDelete")}
                </Button>
              </article>
            ))}
          </div>
        ) : (
          <p className="mini-empty">{t("perf.jfrNoProfiles")}</p>
        )}
        {heap && (
          <div className="heap-card">
            <div className="chart-head">
              <strong>{t("perf.heapTitle")}</strong>
              <span className="muted">
                {heap.collector
                  ? t("perf.heapCollector", { name: heap.collector })
                  : date(heap.at)}
              </span>
            </div>
            {heap.unparsed ? (
              <p className="muted">{t("perf.heapUnparsed")}</p>
            ) : (
              <div className="perf-stats">
                <div>
                  <small>{t("perf.heapUsed")}</small>
                  <strong>{mb(heap.usedMb)}</strong>
                </div>
                <div>
                  <small>{t("perf.heapCommitted")}</small>
                  <strong>{mb(heap.committedMb)}</strong>
                </div>
                <div>
                  <small>{t("perf.heapMax")}</small>
                  <strong>{mb(heap.maxMb)}</strong>
                </div>
                <div>
                  <small>{t("perf.heapMeta")}</small>
                  <strong>{mb(heap.metaspaceUsedMb)}</strong>
                </div>
              </div>
            )}
            <p className="muted">
              <small>
                {t("perf.heapDesc")} · {date(heap.at)}
              </small>
            </p>
          </div>
        )}
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
              <Smartphone size={18} />
              {t("remote.title")}
            </h2>
            <p>{t("remote.desc")}</p>
          </div>
          <div className="task-toggle">
            <button
              type="button"
              className={`switch ${web ? "on" : ""}`}
              role="switch"
              aria-checked={Boolean(web)}
              aria-label={t("remote.enabled")}
              disabled={busy.has(`${key}-web`)}
              onClick={() => void toggleWeb()}
            />
            <span>{t("remote.enabled")}</span>
          </div>
        </div>
        {web ? (
          <div className="remote-console">
            <div className="remote-qr">
              {qr ? (
                <img src={qr} alt={t("remote.scan")} />
              ) : (
                <p className="mini-empty">{t("common.loading")}</p>
              )}
              <small>{t("remote.scan")}</small>
            </div>
            <div className="remote-link">
              <code>{web.url}</code>
              <Button
                busy={busy.has(`${key}-web-copy`)}
                onClick={() => void copyWeb()}
              >
                <Copy size={15} />
                {t("remote.copy")}
              </Button>
              <p className="muted">{t("remote.hint")}</p>
            </div>
          </div>
        ) : (
          <p className="muted">{t("remote.hint")}</p>
        )}
        {webMessage && (
          <p className="success-text" role="status">
            {webMessage}
          </p>
        )}
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
