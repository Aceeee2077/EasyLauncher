import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Archive,
  CalendarDays,
  Clock,
  History,
  Megaphone,
  Pencil,
  Play,
  Plus,
  RotateCw,
  ShieldCheck,
  Square,
  Terminal,
  Trash2,
  Wrench,
} from "lucide-react";
import { Button, Field, Modal, date } from "../components/ui";
import { call, desktop } from "../services/api";
import type {
  MissedRun,
  ScheduledTask,
  ServerView,
  TaskAction,
  TaskCalendarEntry,
  TaskHistoryEntry,
  TaskInput,
  TaskMode,
} from "../types";
import type { TaskProps } from "../utils/task";
import { useI18n, type MessageKey } from "../i18n";

const actions: TaskAction[] = [
  "backup",
  "restart",
  "start",
  "stop",
  "command",
  "cleanup",
  "broadcast",
  "maintenance",
];

const icons = {
  backup: Archive,
  restart: RotateCw,
  start: Play,
  stop: Square,
  command: Terminal,
  cleanup: Trash2,
  broadcast: Megaphone,
  maintenance: Wrench,
} as const;

const cronFields = ["minute", "hour", "day", "month", "weekday"] as const;

const cronPresets = [
  { key: "hourly", expr: "0 * * * *" },
  { key: "daily", expr: "0 4 * * *" },
  { key: "weekly", expr: "0 4 * * 1" },
  { key: "monthly", expr: "0 4 1 * *" },
] as const;

type Draft = {
  action: TaskAction;
  mode: TaskMode;
  everyMinutes: number;
  atTime: string;
  cron: string;
  missedRun: MissedRun;
  dependsOn: string;
  command: string;
  message: string;
  keepDays: number;
  enabled: boolean;
};

const blank: Draft = {
  action: "backup",
  mode: "daily",
  everyMinutes: 720,
  atTime: "04:00",
  cron: "0 4 * * *",
  missedRun: "skip",
  dependsOn: "",
  command: "",
  message: "",
  keepDays: 14,
  enabled: true,
};

function draft(task: ScheduledTask): Draft {
  return {
    action: task.action,
    mode: task.mode,
    everyMinutes: task.everyMinutes || 720,
    atTime: task.atTime || "04:00",
    cron: task.cron || "0 4 * * *",
    missedRun: task.missedRun || "skip",
    dependsOn: task.dependsOn || "",
    command: task.command || "",
    message: task.message || "",
    keepDays: task.keepDays || 14,
    enabled: task.enabled,
  };
}

function payload(value: Draft): TaskInput {
  return {
    action: value.action,
    mode: value.mode,
    everyMinutes: value.everyMinutes,
    atTime: value.mode === "daily" ? value.atTime : null,
    cron: value.mode === "cron" ? value.cron.trim() : null,
    missedRun: value.missedRun,
    dependsOn: value.dependsOn || null,
    command: value.action === "command" ? value.command : null,
    message:
      value.action === "broadcast" || value.action === "maintenance"
        ? value.message
        : null,
    keepDays: value.keepDays,
    enabled: value.enabled,
  };
}

function same(task: ScheduledTask, enabled: boolean): TaskInput {
  return { ...payload(draft(task)), enabled };
}

function cronParts(value: string): string[] {
  const parts = value.trim().split(/\s+/);
  return Array.from({ length: 5 }, (_, index) => parts[index] ?? "*");
}

function errorText(error: unknown): string {
  if (typeof error === "string") return error;
  if (error && typeof error === "object" && "message" in error)
    return String((error as { message: unknown }).message);
  return String(error);
}

export function Schedule({
  server,
  run,
  busy,
}: TaskProps & { server: ServerView }) {
  const { t, locale } = useI18n();
  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const [form, setForm] = useState<{ id: string | null; value: Draft } | null>(
    null,
  );
  const [confirm, setConfirm] = useState<ScheduledTask | null>(null);
  const [message, setMessage] = useState("");
  const [preview, setPreview] = useState<{
    runs: string[];
    error: string | null;
  } | null>(null);
  const [calendarOpen, setCalendarOpen] = useState(false);
  const [calendar, setCalendar] = useState<TaskCalendarEntry[] | null>(null);
  const [historyTask, setHistoryTask] = useState<ScheduledTask | null>(null);
  const [history, setHistory] = useState<TaskHistoryEntry[] | null>(null);
  const load = async () =>
    setTasks(await call("list_tasks", { id: server.id }));
  useEffect(() => {
    void run(`tasks-${server.id}-${crypto.randomUUID()}`, load);
  }, [server.id]);
  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    const pending = listen<{ id?: string }>("schedule-updated", (event) => {
      if (alive && event.payload?.id === server.id) void load();
    });
    return () => {
      alive = false;
      void pending.then((off) => off());
    };
  }, [server.id]);
  useEffect(() => {
    if (!form) {
      setPreview(null);
      return;
    }
    let alive = true;
    const timer = window.setTimeout(() => {
      call("preview_task_schedule", { input: payload(form.value), count: 5 })
        .then((runs) => {
          if (alive) setPreview({ runs, error: null });
        })
        .catch((error: unknown) => {
          if (alive) setPreview({ runs: [], error: errorText(error) });
        });
    }, 350);
    return () => {
      alive = false;
      window.clearTimeout(timer);
    };
  }, [form]);
  useEffect(() => {
    if (!calendarOpen) return;
    let alive = true;
    call("task_calendar", { id: server.id, days: 7 })
      .then((entries) => {
        if (alive) setCalendar(entries);
      })
      .catch(() => {
        if (alive) setCalendar([]);
      });
    return () => {
      alive = false;
    };
  }, [calendarOpen, server.id, tasks]);
  const toggle = (task: ScheduledTask) =>
    void run("taskToggle", async () => {
      await call("update_task", {
        id: server.id,
        taskId: task.id,
        input: same(task, !task.enabled),
      });
      await load();
    });
  const taskLabel = (id: string) => {
    const found = tasks.find((task) => task.id === id);
    return found
      ? t(`schedule.action.${found.action}` as MessageKey)
      : id.slice(0, 8);
  };
  const summary = (task: ScheduledTask) => {
    if (task.action === "command")
      return t("schedule.commandSummary", {
        command: task.command || "",
      });
    if (task.action === "cleanup")
      return t("schedule.cleanupSummary", { n: task.keepDays });
    if (task.action === "broadcast")
      return t("schedule.broadcastSummary", { message: task.message || "" });
    if (task.action === "maintenance")
      return task.message
        ? t("schedule.maintenanceSummary", { message: task.message })
        : t("schedule.maintenanceSummaryBare");
    if (task.mode === "daily")
      return t("schedule.frequencyDaily", { time: task.atTime || "—" });
    if (task.mode === "cron")
      return t("schedule.frequencyCron", { expr: task.cron || "—" });
    return t("schedule.frequencyInterval", { n: task.everyMinutes });
  };
  const resultLabel = (value: string) => {
    if (value === "ok") return t("schedule.success");
    if (value === "missed") return t("schedule.result.missed");
    if (value === "blocked") return t("schedule.result.blocked");
    return t("schedule.failed");
  };
  const resultClass = (value: string) =>
    value === "ok" ? "ok" : value === "missed" || value === "blocked" ? "warn" : "fail";
  const openHistory = (task: ScheduledTask) => {
    setHistoryTask(task);
    setHistory(null);
    void call("task_history", { id: server.id, taskId: task.id })
      .then(setHistory)
      .catch(() => setHistory([]));
  };
  const working = busy.has("taskSave") || busy.has("taskRun");
  return (
    <section className="panel">
      <div className="section-title">
        <div>
          <h2>
            <Clock size={18} />
            {t("schedule.title")}
          </h2>
          <p>{t("schedule.desc")}</p>
        </div>
        <div className="row">
          <Button
            variant="ghost"
            onClick={() => setCalendarOpen((open) => !open)}
          >
            <CalendarDays size={16} />
            {t("schedule.calendar")}
          </Button>
          <Button
            variant="primary"
            onClick={() => {
              setMessage("");
              setForm({ id: null, value: blank });
            }}
          >
            <Plus size={16} />
            {t("schedule.new")}
          </Button>
        </div>
      </div>
      <div className="info-strip">
        <ShieldCheck size={18} />
        <span>{t("schedule.tip")}</span>
      </div>
      {message && (
        <p className="success-text" role="status">
          {message}
        </p>
      )}
      {calendarOpen && (
        <div className="task-calendar">
          {Array.from({ length: 7 }, (_, offset) => {
            const day = new Date();
            day.setHours(0, 0, 0, 0);
            day.setDate(day.getDate() + offset);
            const next = new Date(day);
            next.setDate(next.getDate() + 1);
            const items = (calendar ?? []).filter((entry) => {
              const at = new Date(entry.at);
              return at >= day && at < next;
            });
            return (
              <div className="task-calendar-day" key={day.toISOString()}>
                <div className="task-calendar-date">
                  <strong>
                    {day.toLocaleDateString(locale, {
                      month: "numeric",
                      day: "numeric",
                    })}
                  </strong>
                  <small>
                    {day.toLocaleDateString(locale, { weekday: "short" })}
                  </small>
                </div>
                {items.length ? (
                  items.map((entry, index) => (
                    <div
                      className="task-calendar-entry"
                      key={`${entry.taskId}-${entry.at}-${index}`}
                    >
                      <span>
                        {new Date(entry.at).toLocaleTimeString(locale, {
                          hour: "2-digit",
                          minute: "2-digit",
                        })}
                      </span>
                      <small>
                        {t(`schedule.action.${entry.action}` as MessageKey)}
                      </small>
                    </div>
                  ))
                ) : (
                  <small className="task-calendar-empty">—</small>
                )}
              </div>
            );
          })}
        </div>
      )}
      {tasks.length ? (
        <div className="task-list">
          {tasks.map((task) => {
            const Icon = icons[task.action];
            return (
              <div
                className={`task-row ${task.enabled ? "" : "paused"}`}
                key={task.id}
              >
                <div className="task-main">
                  <span className="task-icon">
                    <Icon size={17} />
                  </span>
                  <div>
                    <strong>
                      {t(`schedule.action.${task.action}` as MessageKey)}
                    </strong>
                    <small>{summary(task)}</small>
                    {task.dependsOn && (
                      <small className="task-dependency">
                        {t("schedule.dependsOn", {
                          task: taskLabel(task.dependsOn),
                        })}
                      </small>
                    )}
                  </div>
                </div>
                <div className="task-when">
                  <small>{t("schedule.nextRun")}</small>
                  <strong>{task.enabled ? date(task.nextRunAt) : "—"}</strong>
                </div>
                <div className="task-when task-note">
                  <small>{t("schedule.lastRun")}</small>
                  <span>
                    {task.lastRunAt ? date(task.lastRunAt) : t("schedule.never")}
                  </span>
                  {task.lastResult && (
                    <em
                      className={resultClass(task.lastResult)}
                      title={task.lastResult}
                    >
                      {resultLabel(task.lastResult)}
                    </em>
                  )}
                </div>
                <div className="row">
                  <button
                    className={`switch ${task.enabled ? "on" : ""}`}
                    role="switch"
                    aria-checked={task.enabled}
                    aria-label={t("schedule.enableAria")}
                    disabled={working}
                    onClick={() => toggle(task)}
                  />
                  <Button
                    variant="ghost"
                    disabled={working}
                    aria-label={t("schedule.runNow")}
                    title={t("schedule.runNow")}
                    onClick={() =>
                      void run("taskRun", async () => {
                        await call("run_task_now", {
                          id: server.id,
                          taskId: task.id,
                        });
                        setMessage(t("schedule.ran"));
                        await load();
                      })
                    }
                  >
                    <Play size={16} />
                  </Button>
                  <Button
                    variant="ghost"
                    aria-label={t("schedule.history")}
                    title={t("schedule.history")}
                    onClick={() => openHistory(task)}
                  >
                    <History size={16} />
                  </Button>
                  <Button
                    variant="ghost"
                    disabled={working}
                    aria-label={t("schedule.editTitle")}
                    title={t("schedule.editTitle")}
                    onClick={() => {
                      setMessage("");
                      setForm({ id: task.id, value: draft(task) });
                    }}
                  >
                    <Pencil size={16} />
                  </Button>
                  <Button
                    variant="ghost"
                    disabled={working}
                    aria-label={t("schedule.delete")}
                    title={t("schedule.delete")}
                    onClick={() => setConfirm(task)}
                  >
                    <Trash2 size={16} />
                  </Button>
                </div>
              </div>
            );
          })}
        </div>
      ) : (
        <div className="mini-empty">
          <Clock size={36} />
          <h3>{t("schedule.emptyTitle")}</h3>
          <p>{t("schedule.emptyDesc")}</p>
        </div>
      )}
      {form && (
        <Modal
          title={form.id ? t("schedule.editTitle") : t("schedule.createTitle")}
          onClose={() => {
            if (!busy.has("taskSave")) setForm(null);
          }}
        >
          <div className="form-grid">
            <Field label={t("schedule.action")}>
              <select
                value={form.value.action}
                onChange={(e) =>
                  setForm({
                    ...form,
                    value: {
                      ...form.value,
                      action: e.target.value as TaskAction,
                    },
                  })
                }
              >
                {actions.map((action) => (
                  <option value={action} key={action}>
                    {t(`schedule.action.${action}` as MessageKey)}
                  </option>
                ))}
              </select>
            </Field>
            <Field label={t("schedule.mode")}>
              <select
                value={form.value.mode}
                onChange={(e) =>
                  setForm({
                    ...form,
                    value: { ...form.value, mode: e.target.value as TaskMode },
                  })
                }
              >
                <option value="interval">{t("schedule.mode.interval")}</option>
                <option value="daily">{t("schedule.mode.daily")}</option>
                <option value="cron">{t("schedule.mode.cron")}</option>
              </select>
            </Field>
            {form.value.mode === "daily" && (
              <Field label={t("schedule.at")}>
                <input
                  type="time"
                  value={form.value.atTime}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      value: { ...form.value, atTime: e.target.value },
                    })
                  }
                />
              </Field>
            )}
            {form.value.mode === "interval" && (
              <Field label={t("schedule.every")} hint={t("schedule.everyHint")}>
                <input
                  type="number"
                  min={5}
                  max={10080}
                  value={form.value.everyMinutes}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      value: {
                        ...form.value,
                        everyMinutes: Number(e.target.value),
                      },
                    })
                  }
                />
              </Field>
            )}
            {form.value.mode === "cron" && (
              <div className="cron-editor">
                <div className="cron-preset">
                  <label>{t("schedule.cron.preset")}</label>
                  <select
                    value={
                      cronPresets.find(
                        (preset) => preset.expr === form.value.cron,
                      )?.expr ?? "custom"
                    }
                    onChange={(e) => {
                      if (e.target.value !== "custom")
                        setForm({
                          ...form,
                          value: { ...form.value, cron: e.target.value },
                        });
                    }}
                  >
                    <option value="custom">{t("schedule.cron.custom")}</option>
                    {cronPresets.map((preset) => (
                      <option value={preset.expr} key={preset.key}>
                        {t(`schedule.cron.preset.${preset.key}` as MessageKey)}
                      </option>
                    ))}
                  </select>
                </div>
                <div className="cron-fields">
                  {cronFields.map((field, index) => (
                    <label key={field}>
                      <span>
                        {t(`schedule.cron.${field}` as MessageKey)}
                      </span>
                      <input
                        value={cronParts(form.value.cron)[index]}
                        placeholder={index === 0 ? "0" : "*"}
                        onChange={(e) => {
                          const parts = cronParts(form.value.cron);
                          parts[index] = e.target.value;
                          setForm({
                            ...form,
                            value: { ...form.value, cron: parts.join(" ") },
                          });
                        }}
                      />
                    </label>
                  ))}
                </div>
                <small className="cron-hint">{t("schedule.cron.hint")}</small>
              </div>
            )}
            <div className="task-preview">
              <small>{t("schedule.preview")}</small>
              {preview === null ? (
                <span>{t("common.loading")}</span>
              ) : preview.error ? (
                <p className="task-preview-error">{preview.error}</p>
              ) : (
                <ul>
                  {preview.runs.map((runAt) => (
                    <li key={runAt}>{date(runAt)}</li>
                  ))}
                </ul>
              )}
            </div>
            <Field label={t("schedule.missedRun")} hint={t("schedule.missedRunHint")}>
              <select
                value={form.value.missedRun}
                onChange={(e) =>
                  setForm({
                    ...form,
                    value: {
                      ...form.value,
                      missedRun: e.target.value as MissedRun,
                    },
                  })
                }
              >
                <option value="skip">{t("schedule.missedRun.skip")}</option>
                <option value="catchUp">
                  {t("schedule.missedRun.catchUp")}
                </option>
              </select>
            </Field>
            <Field
              label={t("schedule.dependency")}
              hint={t("schedule.dependencyHint")}
            >
              <select
                value={form.value.dependsOn}
                onChange={(e) =>
                  setForm({
                    ...form,
                    value: { ...form.value, dependsOn: e.target.value },
                  })
                }
              >
                <option value="">{t("schedule.dependencyNone")}</option>
                {tasks
                  .filter((task) => task.id !== form.id)
                  .map((task) => (
                    <option value={task.id} key={task.id}>
                      {t(`schedule.action.${task.action}` as MessageKey)} ·{" "}
                      {summary(task)}
                    </option>
                  ))}
              </select>
            </Field>
            {form.value.action === "command" && (
              <Field
                label={t("schedule.command")}
                hint={t("schedule.commandHint")}
              >
                <input
                  value={form.value.command}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      value: { ...form.value, command: e.target.value },
                    })
                  }
                />
              </Field>
            )}
            {form.value.action === "cleanup" && (
              <Field
                label={t("schedule.keepDays")}
                hint={t("schedule.keepDaysHint")}
              >
                <input
                  type="number"
                  min={1}
                  max={365}
                  value={form.value.keepDays}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      value: {
                        ...form.value,
                        keepDays: Number(e.target.value),
                      },
                    })
                  }
                />
              </Field>
            )}
            {(form.value.action === "broadcast" ||
              form.value.action === "maintenance") && (
              <Field
                label={t("schedule.message")}
                hint={t("schedule.messageHint")}
              >
                <input
                  maxLength={256}
                  value={form.value.message}
                  onChange={(e) =>
                    setForm({
                      ...form,
                      value: { ...form.value, message: e.target.value },
                    })
                  }
                />
              </Field>
            )}
            <div className="task-toggle">
              <button
                type="button"
                className={`switch ${form.value.enabled ? "on" : ""}`}
                role="switch"
                aria-checked={form.value.enabled}
                aria-label={t("schedule.enabled")}
                onClick={() =>
                  setForm({
                    ...form,
                    value: { ...form.value, enabled: !form.value.enabled },
                  })
                }
              />
              <span>{t("schedule.enabled")}</span>
            </div>
          </div>
          <div className="modal-actions">
            <Button
              onClick={() => setForm(null)}
              disabled={busy.has("taskSave")}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              busy={busy.has("taskSave")}
              onClick={() =>
                void run("taskSave", async () => {
                  const input = payload(form.value);
                  await (form.id
                    ? call("update_task", {
                        id: server.id,
                        taskId: form.id,
                        input,
                      })
                    : call("create_task", { id: server.id, input }));
                  setForm(null);
                  setMessage(t("schedule.saved"));
                  await load();
                })
              }
            >
              {t("schedule.save")}
            </Button>
          </div>
        </Modal>
      )}
      {historyTask && (
        <Modal
          title={t("schedule.historyTitle", {
            task: t(`schedule.action.${historyTask.action}` as MessageKey),
          })}
          onClose={() => setHistoryTask(null)}
        >
          {history === null ? (
            <p>{t("common.loading")}</p>
          ) : history.length ? (
            <div className="task-history">
              {history.map((entry, index) => (
                <div
                  className="task-history-row"
                  key={`${entry.at}-${entry.trigger}-${index}`}
                >
                  <div>
                    <strong>{date(entry.at)}</strong>
                    <small>
                      {entry.trigger === "manual"
                        ? t("schedule.trigger.manual")
                        : t("schedule.trigger.schedule")}
                    </small>
                  </div>
                  <em className={resultClass(entry.result)}>
                    {resultLabel(entry.result)}
                  </em>
                  {entry.detail && <p>{entry.detail}</p>}
                </div>
              ))}
            </div>
          ) : (
            <p>{t("schedule.historyEmpty")}</p>
          )}
        </Modal>
      )}
      {confirm && (
        <Modal
          title={t("schedule.deleteTitle")}
          onClose={() => {
            if (!busy.has("taskDelete")) setConfirm(null);
          }}
        >
          <p>{t("schedule.deleteBody")}</p>
          <div className="modal-actions">
            <Button
              onClick={() => setConfirm(null)}
              disabled={busy.has("taskDelete")}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              busy={busy.has("taskDelete")}
              onClick={() =>
                void run("taskDelete", async () => {
                  await call("delete_task", {
                    id: server.id,
                    taskId: confirm.id,
                  });
                  setConfirm(null);
                  setMessage(t("schedule.deleted"));
                  await load();
                })
              }
            >
              {t("schedule.delete")}
            </Button>
          </div>
        </Modal>
      )}
    </section>
  );
}
