import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Activity,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  CirclePause,
  MemoryStick,
  MessageSquare,
  Network,
  Play,
  RefreshCw,
  RotateCw,
  Server as ServerIcon,
  ShieldCheck,
  Square,
  TriangleAlert,
  UserMinus,
  UserRound,
  Users,
  X,
} from "lucide-react";
import { Button, StatusBadge, date } from "../components/ui";
import { useI18n, type MessageKey } from "../i18n";
import { errorMessageKey } from "../i18n/messages";
import { call, desktop } from "../services/api";
import {
  active,
  idle,
  type AppError,
  type PortIssue,
  type ResourceReport,
  type ServerView,
  type StartPlan,
  type StartQueue,
} from "../types";
import type { TaskProps } from "../utils/task";

type Action = "start_server" | "stop_server" | "restart_server";

/** Only offer an action when the server can actually accept it. */
const accepts = (action: Action, server: ServerView) => {
  const status = server.runtime.status;
  if (action === "start_server") return server.installed && idle(status);
  if (action === "stop_server")
    return active(status) && status !== "stopping";
  return server.installed && ["running", "crashed", "stopped"].includes(status);
};

const actionLabel = {
  start_server: "overview.action.start",
  stop_server: "overview.action.stop",
  restart_server: "overview.action.restart",
} as const;

type Outcome = {
  action: Action;
  ok: number;
  failed: { name: string; message: string }[];
};

const sizeLabel = (mb: number) =>
  mb >= 1024 ? `${(mb / 1024).toFixed(1)} GB` : `${Math.round(mb)} MB`;

export function Overview({
  servers,
  run,
  busy,
  reload,
  onManage,
}: TaskProps & {
  servers: ServerView[];
  reload: () => Promise<void>;
  onManage: (server: ServerView) => void;
}) {
  const { t } = useI18n();
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [outcome, setOutcome] = useState<Outcome | null>(null);
  const [openPlayers, setOpenPlayers] = useState<Set<string>>(new Set());
  const [chat, setChat] = useState<{
    serverId: string;
    player: string;
  } | null>(null);
  const [draft, setDraft] = useState("");
  const [sent, setSent] = useState<Record<string, string>>({});
  const [report, setReport] = useState<ResourceReport | null>(null);
  const [queue, setQueue] = useState<StartQueue | null>(null);
  const [plan, setPlan] = useState<StartPlan | null>(null);
  const [reportError, setReportError] = useState<string | null>(null);
  const [stagger, setStagger] = useState(10);
  const [waitReady, setWaitReady] = useState(60);
  const [force, setForce] = useState(false);
  const running = servers.filter((s) => s.runtime.status === "running").length;
  const idleCount = servers.filter((s) => idle(s.runtime.status)).length;
  const attention = servers.filter((s) =>
    ["crashed", "installationFailed"].includes(s.runtime.status),
  ).length;
  const statusKey = servers
    .map((s) => `${s.id}:${s.runtime.status}`)
    .join("|");
  const selectionKey = [...selected].join(",");
  const loadReport = useCallback(async () => {
    try {
      const next = await call("resources_report");
      setReport(next);
      setQueue(next.queue);
      setReportError(null);
    } catch (e) {
      setReportError((e as Partial<AppError>)?.message || String(e));
    }
  }, []);
  useEffect(() => {
    if (!desktop) return;
    void loadReport();
  }, [desktop, loadReport, statusKey]);
  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    const subscription = listen<StartQueue>("start-queue", (event) => {
      if (!alive) return;
      setQueue(event.payload);
      if (!event.payload.running) {
        void loadReport();
        void reload();
      }
    });
    return () => {
      alive = false;
      void subscription.then((off) => off());
    };
  }, [desktop, loadReport, reload]);
  useEffect(() => {
    if (!desktop || !selectionKey) {
      setPlan(null);
      return;
    }
    let alive = true;
    const timer = window.setTimeout(() => {
      void call("resources_plan", {
        ids: selectionKey.split(","),
        options: { staggerSecs: stagger, waitReadySecs: waitReady, force },
      })
        .then((next) => {
          if (alive) setPlan(next);
        })
        .catch(() => {
          if (alive) setPlan(null);
        });
    }, 120);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [desktop, selectionKey, stagger, waitReady, force, statusKey]);
  const targets = (action: Action) =>
    servers.filter((s) => selected.has(s.id) && accepts(action, s));
  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  const togglePlayers = (id: string) =>
    setOpenPlayers((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });
  const remember = (id: string, text: string) =>
    setSent((prev) => ({ ...prev, [id]: text }));
  const act = (server: ServerView, player: string, action: "kick" | "op") =>
    void run(`overview-player-${server.id}`, async () => {
      await call("player_action", { id: server.id, player, action });
      remember(
        server.id,
        t("overview.playerDone", {
          action: t(action === "kick" ? "overview.kick" : "overview.op"),
          player,
        }),
      );
    });
  const sendMessage = (server: ServerView) => {
    if (!chat || chat.serverId !== server.id) return;
    const text = draft.trim();
    if (!text) return;
    void run(`overview-chat-${server.id}`, async () => {
      await call("send_console_command", {
        id: server.id,
        command: `tell ${chat.player} ${text}`,
      });
      remember(server.id, t("overview.msgDone", { player: chat.player }));
      setChat(null);
      setDraft("");
    });
  };
  const allSelected =
    servers.length > 0 && servers.every((s) => selected.has(s.id));
  const perform = (action: Action) => {
    const list = targets(action);
    if (!list.length) return;
    void run("overview-batch", async () => {
      const results = await Promise.allSettled(
        list.map((s) => call(action, { id: s.id })),
      );
      const failed = results.flatMap((result, i) =>
        result.status === "rejected"
          ? [
              {
                name: list[i].name,
                message:
                  (result.reason as Partial<AppError>)?.message ||
                  String(result.reason),
              },
            ]
          : [],
      );
      setOutcome({ action, ok: list.length - failed.length, failed });
      setSelected(new Set());
      await reload();
    });
  };
  const queueStart = () => {
    const ids = [...selected];
    if (!ids.length) return;
    void run("overview-queue", async () => {
      const next = await call("resources_start_queue", {
        ids,
        options: { staggerSecs: stagger, waitReadySecs: waitReady, force },
      });
      setQueue(next);
      setSelected(new Set());
      await loadReport();
    });
  };
  const queueCancel = () => {
    void run("overview-queue-cancel", async () => {
      setQueue(await call("resources_cancel_queue"));
    });
  };
  const applyPort = (issue: PortIssue) => {
    if (!issue.suggestion) return;
    const port = issue.suggestion;
    void run(`overview-port-${issue.id}`, async () => {
      await call("resources_apply_port", { id: issue.id, port });
      await reload();
      await loadReport();
    });
  };
  const reasonLabel = (code: string | null) => {
    if (!code) return "";
    const key = errorMessageKey(code);
    return key ? t(key) : code;
  };
  const stepLabel = (step: string) =>
    t(`overview.step.${step}` as MessageKey);
  const buttons = [
    ["start_server", "overview.start", Play],
    ["stop_server", "overview.stop", Square],
    ["restart_server", "overview.restart", RotateCw],
  ] as const;
  const budgetMb = report?.budgetMb ?? 0;
  const projectedMb = report?.projectedMb ?? 0;
  const scale = Math.max(budgetMb, projectedMb, 1);
  const width = (value: number) =>
    `${Math.min(100, (value / scale) * 100).toFixed(2)}%`;
  const freeMb = Math.max(0, budgetMb - projectedMb);
  return (
    <div className="ops-stack">
      <div className="stats-grid">
        <div className="stat">
          <span className="stat-icon">
            <ServerIcon size={21} />
          </span>
          <div>
            <small>{t("overview.total")}</small>
            <strong>{servers.length.toString().padStart(2, "0")}</strong>
          </div>
          <span className="stat-note">{t("overview.totalNote")}</span>
        </div>
        <div className="stat">
          <span className="stat-icon green">
            <Activity size={21} />
          </span>
          <div>
            <small>{t("overview.running")}</small>
            <strong>{running.toString().padStart(2, "0")}</strong>
          </div>
          <span className="stat-note">{t("overview.runningNote")}</span>
        </div>
        <div className="stat">
          <span className="stat-icon">
            <CirclePause size={21} />
          </span>
          <div>
            <small>{t("overview.idle")}</small>
            <strong>{idleCount.toString().padStart(2, "0")}</strong>
          </div>
          <span className="stat-note">{t("overview.idleNote")}</span>
        </div>
        <div className="stat">
          <span className="stat-icon">
            <TriangleAlert size={21} />
          </span>
          <div>
            <small>{t("overview.attention")}</small>
            <strong>{attention.toString().padStart(2, "0")}</strong>
          </div>
          <span className="stat-note">{t("overview.attentionNote")}</span>
        </div>
      </div>
      {report && (
        <section className="panel">
          <div className="section-title">
            <div>
              <h2>{t("overview.resource.title")}</h2>
              <p>{t("overview.resource.desc")}</p>
            </div>
            <button
              className="player-action"
              onClick={() => void loadReport()}
            >
              <RefreshCw size={13} />
              {t("common.refresh")}
            </button>
          </div>
          <div className="resource-memory">
            <div
              className={`resource-bar ${report.budgetOk ? "" : "over"}`}
              role="img"
              aria-label={t("overview.resource.title")}
            >
              <span
                className="running"
                style={{ width: width(report.runningMb) }}
              />
              <span className="idle" style={{ width: width(report.idleMb) }} />
              <span className="free" style={{ width: width(freeMb) }} />
            </div>
            <div className="resource-legend">
              <span>
                <i className="running" />
                {t("overview.resource.running")} {sizeLabel(report.runningMb)}
              </span>
              <span>
                <i className="idle" />
                {t("overview.resource.idle")} {sizeLabel(report.idleMb)}
              </span>
              <span>
                <i className="free" />
                {t("overview.resource.free")} {sizeLabel(freeMb)}
              </span>
            </div>
          </div>
          <div className="perf-stats">
            <div>
              <small>{t("overview.resource.budget")}</small>
              <strong>{sizeLabel(report.budgetMb)}</strong>
              <em>
                {t("overview.resource.available", {
                  size: sizeLabel(report.availableMb),
                })}
              </em>
            </div>
            <div>
              <small>{t("overview.resource.running")}</small>
              <strong>{sizeLabel(report.runningMb)}</strong>
            </div>
            <div>
              <small>{t("overview.resource.idle")}</small>
              <strong>{sizeLabel(report.idleMb)}</strong>
            </div>
            <div>
              <small>{t("overview.resource.projected")}</small>
              <strong>{sizeLabel(report.projectedMb)}</strong>
              <em>
                {t("overview.resource.free")} {sizeLabel(freeMb)}
              </em>
            </div>
          </div>
          <p
            className={`resource-status ${report.budgetOk ? "" : "bad"}`}
            role="status"
          >
            {report.budgetOk
              ? t("overview.resource.budgetOk")
              : t("overview.resource.overBudget")}
            {report.tight ? ` · ${t("overview.resource.tight")}` : ""}
          </p>
          <div className="resource-grid">
            <div className="resource-section">
              <h3>
                <Network size={14} />
                {t("overview.resource.ports")}
              </h3>
              {report.ports.length ? (
                <ul className="resource-port-list">
                  {report.ports.map((issue) => (
                    <li key={`${issue.id}-${issue.kind}`}>
                      <TriangleAlert size={14} />
                      <div>
                        <strong>{issue.name}</strong>
                        <small>
                          {t(
                            issue.kind === "duplicate"
                              ? "overview.resource.duplicate"
                              : "overview.resource.occupied",
                          )}{" "}
                          · :{issue.port}
                        </small>
                      </div>
                      {issue.suggestion && (
                        <Button
                          busy={busy.has(`overview-port-${issue.id}`)}
                          onClick={() => applyPort(issue)}
                        >
                          {t("overview.resource.applyPort", {
                            port: issue.suggestion,
                          })}
                        </Button>
                      )}
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="success-text small">
                  {t("overview.resource.noPortIssues")}
                </p>
              )}
              <p className="overview-players-note">
                {t("overview.resource.portHint")}
              </p>
            </div>
            <div className="resource-section">
              <h3>
                <MemoryStick size={14} />
                {t("overview.resource.queue")}
              </h3>
              <div className="resource-options">
                <label>
                  <span>{t("overview.resource.stagger")}</span>
                  <input
                    type="number"
                    min={0}
                    max={120}
                    value={stagger}
                    onChange={(e) =>
                      setStagger(
                        Math.max(0, Math.min(120, Number(e.target.value) || 0)),
                      )
                    }
                  />
                </label>
                <label>
                  <span>{t("overview.resource.waitReady")}</span>
                  <input
                    type="number"
                    min={0}
                    max={180}
                    value={waitReady}
                    onChange={(e) =>
                      setWaitReady(
                        Math.max(0, Math.min(180, Number(e.target.value) || 0)),
                      )
                    }
                  />
                </label>
                <label className="overview-check">
                  <input
                    type="checkbox"
                    checked={force}
                    onChange={(e) => setForce(e.target.checked)}
                  />
                  <span>{t("overview.resource.force")}</span>
                </label>
              </div>
              {plan ? (
                <div className="resource-plan">
                  <p className="perf-subtitle">
                    {t("overview.resource.plan")} ·{" "}
                    {t("overview.resource.summary", {
                      count: plan.startable,
                      size: sizeLabel(plan.projectedMb),
                    })}
                  </p>
                  <ul className="resource-plan-list">
                    {plan.entries.map((entry) => (
                      <li
                        key={entry.id}
                        className={entry.ready ? "" : "skipped"}
                      >
                        <strong>{entry.name}</strong>
                        <small>
                          {sizeLabel(entry.memoryMb)} · :{entry.port}
                        </small>
                        {entry.ready ? (
                          <em>
                            {entry.startAfterSecs
                              ? t("overview.resource.afterSecs", {
                                  secs: entry.startAfterSecs,
                                })
                              : t("overview.resource.planReady")}
                          </em>
                        ) : (
                          <em className="warn-text">
                            {reasonLabel(entry.reason)}
                          </em>
                        )}
                      </li>
                    ))}
                  </ul>
                  {!plan.budgetOk && (
                    <p className="warn-text">
                      {t("overview.resource.overBudget")}
                    </p>
                  )}
                </div>
              ) : (
                <p className="overview-players-note">
                  {t("overview.resource.planEmpty")}
                </p>
              )}
              <div className="row">
                <Button
                  variant="primary"
                  disabled={
                    !plan?.startable ||
                    !!queue?.running ||
                    (!!plan && !plan.budgetOk && !force)
                  }
                  busy={busy.has("overview-queue")}
                  onClick={queueStart}
                >
                  <Play size={14} />
                  {t("overview.resource.start", {
                    count: plan?.startable ?? 0,
                  })}
                </Button>
                {queue?.running && (
                  <Button
                    busy={busy.has("overview-queue-cancel")}
                    onClick={queueCancel}
                  >
                    <X size={14} />
                    {t("overview.resource.cancel")}
                  </Button>
                )}
              </div>
              {queue && (
                <div className="resource-queue-state">
                  <p className="perf-subtitle">
                    {date(queue.startedAt)} · {sizeLabel(queue.memoryMb)}
                  </p>
                  <ul className="resource-plan-list">
                    {queue.entries.map((entry) => (
                      <li key={entry.id} className={`step-${entry.step}`}>
                        <strong>{entry.name}</strong>
                        <span className="resource-step">
                          {stepLabel(entry.step)}
                        </span>
                        {entry.detail && <small>{entry.detail}</small>}
                      </li>
                    ))}
                  </ul>
                </div>
              )}
              <p className="overview-players-note">
                {t("overview.resource.queueHint")}
              </p>
            </div>
          </div>
        </section>
      )}
      {reportError && (
        <p className="warn-text">
          {t("overview.resource.error", { message: reportError })}
        </p>
      )}
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>{t("overview.listTitle")}</h2>
            <p>{t("overview.listDesc")}</p>
          </div>
        </div>
        <div className="overview-toolbar">
          <label className="overview-check">
            <input
              type="checkbox"
              checked={allSelected}
              disabled={!servers.length}
              onChange={() =>
                setSelected(
                  allSelected ? new Set() : new Set(servers.map((s) => s.id)),
                )
              }
            />
            <span>{t("overview.selectAll")}</span>
          </label>
          <div className="row">
            {buttons.map(([action, label, Icon]) => {
              const count = targets(action).length;
              return (
                <Button
                  key={action}
                  variant={action === "start_server" ? "primary" : "secondary"}
                  disabled={!count}
                  busy={busy.has("overview-batch")}
                  onClick={() => perform(action)}
                >
                  <Icon size={14} />
                  {t(label, { count })}
                </Button>
              );
            })}
          </div>
        </div>
        {outcome && (
          <div
            className={`overview-result ${outcome.failed.length ? "bad" : ""}`}
          >
            <span>
              {t("overview.result", {
                action: t(actionLabel[outcome.action]),
                ok: outcome.ok,
                failed: outcome.failed.length,
              })}
            </span>
            {outcome.failed.length > 0 && (
              <ul className="overview-failed">
                {outcome.failed.map((item) => (
                  <li key={item.name}>{t("overview.failure", item)}</li>
                ))}
              </ul>
            )}
          </div>
        )}
        {servers.length ? (
          <div className="overview-list">
            {servers.map((s) => {
              const online = openPlayers.has(s.id);
              const busyKey = `overview-player-${s.id}`;
              return (
                <div className="overview-item" key={s.id}>
                  <div className="overview-row">
                    <input
                      type="checkbox"
                      aria-label={t("overview.select", { name: s.name })}
                      checked={selected.has(s.id)}
                      onChange={() => toggle(s.id)}
                    />
                    <div className="overview-main">
                      <strong>
                        {s.name}
                        <StatusBadge status={s.runtime.status} />
                      </strong>
                      <p>
                        {t("overview.meta", {
                          type: s.serverType,
                          version: s.minecraftVersion,
                          port: s.port,
                        })}
                        {s.runtime.status === "running"
                          ? ` · ${t("overview.players", {
                              count: s.runtime.players.length,
                            })}`
                          : ""}
                      </p>
                    </div>
                    {s.runtime.status === "running" && (
                      <Button
                        onClick={() => togglePlayers(s.id)}
                        aria-expanded={online}
                      >
                        <Users size={14} />
                        {t("overview.online", {
                          count: s.runtime.players.length,
                        })}
                        {online ? (
                          <ChevronUp size={14} />
                        ) : (
                          <ChevronDown size={14} />
                        )}
                      </Button>
                    )}
                    <Button onClick={() => onManage(s)}>
                      {t("common.manage")}
                      <ChevronRight size={14} />
                    </Button>
                  </div>
                  {online && s.runtime.status === "running" && (
                    <div className="overview-players">
                      <div className="row">
                        <button
                          className="player-action"
                          disabled={busy.has(`${busyKey}-rcon`)}
                          onClick={() =>
                            void run(`${busyKey}-rcon`, async () => {
                              await call("rcon_players", { id: s.id });
                              await reload();
                            })
                          }
                        >
                          <RefreshCw size={13} />
                          {t("overview.rconRefresh")}
                        </button>
                      </div>
                      {s.runtime.players.length ? (
                        s.runtime.players.map((player) => {
                          const chatting =
                            chat?.serverId === s.id && chat.player === player;
                          return (
                            <div className="overview-player" key={player}>
                              <span className="avatar">
                                <UserRound size={15} />
                              </span>
                              <strong>{player}</strong>
                              <div className="row">
                                <button
                                  className="player-action"
                                  disabled={busy.has(busyKey)}
                                  onClick={() => act(s, player, "kick")}
                                >
                                  <UserMinus size={13} />
                                  {t("overview.kick")}
                                </button>
                                <button
                                  className="player-action"
                                  disabled={busy.has(busyKey)}
                                  onClick={() => act(s, player, "op")}
                                >
                                  <ShieldCheck size={13} />
                                  {t("overview.op")}
                                </button>
                                <button
                                  className={`player-action${chatting ? " active" : ""}`}
                                  onClick={() => {
                                    setChat(
                                      chatting
                                        ? null
                                        : { serverId: s.id, player },
                                    );
                                    setDraft("");
                                  }}
                                >
                                  <MessageSquare size={13} />
                                  {t("overview.message")}
                                </button>
                              </div>
                              {chatting && (
                                <form
                                  className="overview-chat"
                                  onSubmit={(e) => {
                                    e.preventDefault();
                                    sendMessage(s);
                                  }}
                                >
                                  <input
                                    autoFocus
                                    maxLength={256}
                                    required
                                    value={draft}
                                    placeholder={t(
                                      "overview.messagePlaceholder",
                                      { player },
                                    )}
                                    onChange={(e) => setDraft(e.target.value)}
                                  />
                                  <Button
                                    variant="primary"
                                    type="submit"
                                    busy={busy.has(
                                      `overview-chat-${s.id}`,
                                    )}
                                    disabled={!draft.trim()}
                                  >
                                    {t("overview.send")}
                                  </Button>
                                  <button
                                    type="button"
                                    className="player-action"
                                    onClick={() => setChat(null)}
                                  >
                                    {t("common.cancel")}
                                  </button>
                                </form>
                              )}
                            </div>
                          );
                        })
                      ) : (
                        <p className="overview-players-empty">
                          {t("overview.noPlayers")}
                        </p>
                      )}
                      <p className="overview-players-note">
                        {t("overview.playerNote")}
                      </p>
                      <p className="overview-players-note">
                        {t("overview.rconNote")}
                      </p>
                      {sent[s.id] && (
                        <p className="success-text small" role="status">
                          {sent[s.id]}
                        </p>
                      )}
                    </div>
                  )}
                </div>
              );
            })}
          </div>
        ) : (
          <p className="overview-empty">{t("overview.empty")}</p>
        )}
      </section>
    </div>
  );
}
