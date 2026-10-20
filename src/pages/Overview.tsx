import { useState } from "react";
import {
  Activity,
  ChevronRight,
  CirclePause,
  Play,
  RotateCw,
  Server as ServerIcon,
  Square,
  TriangleAlert,
} from "lucide-react";
import { Button, StatusBadge } from "../components/ui";
import { useI18n } from "../i18n";
import { call } from "../services/api";
import { active, idle, type AppError, type ServerView } from "../types";
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
  const running = servers.filter((s) => s.runtime.status === "running").length;
  const idleCount = servers.filter((s) => idle(s.runtime.status)).length;
  const attention = servers.filter((s) =>
    ["crashed", "installationFailed"].includes(s.runtime.status),
  ).length;
  const targets = (action: Action) =>
    servers.filter((s) => selected.has(s.id) && accepts(action, s));
  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev);
      if (!next.delete(id)) next.add(id);
      return next;
    });
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
  const buttons = [
    ["start_server", "overview.start", Play],
    ["stop_server", "overview.stop", Square],
    ["restart_server", "overview.restart", RotateCw],
  ] as const;
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
            {servers.map((s) => (
              <div className="overview-row" key={s.id}>
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
                <Button onClick={() => onManage(s)}>
                  {t("common.manage")}
                  <ChevronRight size={14} />
                </Button>
              </div>
            ))}
          </div>
        ) : (
          <p className="overview-empty">{t("overview.empty")}</p>
        )}
      </section>
    </div>
  );
}
