import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Archive,
  Clock,
  Pencil,
  Play,
  Plus,
  RotateCw,
  ShieldCheck,
  Square,
  Terminal,
  Trash2,
} from "lucide-react";
import { Button, Field, Modal, date } from "../components/ui";
import { call, desktop } from "../services/api";
import type {
  ScheduledTask,
  ServerView,
  TaskAction,
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
];

const icons = {
  backup: Archive,
  restart: RotateCw,
  start: Play,
  stop: Square,
  command: Terminal,
  cleanup: Trash2,
} as const;

type Draft = {
  action: TaskAction;
  mode: TaskMode;
  everyMinutes: number;
  atTime: string;
  command: string;
  keepDays: number;
  enabled: boolean;
};

const blank: Draft = {
  action: "backup",
  mode: "daily",
  everyMinutes: 720,
  atTime: "04:00",
  command: "",
  keepDays: 14,
  enabled: true,
};

function draft(task: ScheduledTask): Draft {
  return {
    action: task.action,
    mode: task.mode,
    everyMinutes: task.everyMinutes || 720,
    atTime: task.atTime || "04:00",
    command: task.command || "",
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
    command: value.action === "command" ? value.command : null,
    keepDays: value.keepDays,
    enabled: value.enabled,
  };
}

function same(task: ScheduledTask, enabled: boolean): TaskInput {
  return { ...payload(draft(task)), enabled };
}

export function Schedule({
  server,
  run,
  busy,
}: TaskProps & { server: ServerView }) {
  const { t } = useI18n();
  const [tasks, setTasks] = useState<ScheduledTask[]>([]);
  const [form, setForm] = useState<{ id: string | null; value: Draft } | null>(
    null,
  );
  const [confirm, setConfirm] = useState<ScheduledTask | null>(null);
  const [message, setMessage] = useState("");
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
  const toggle = (task: ScheduledTask) =>
    void run("taskToggle", async () => {
      await call("update_task", {
        id: server.id,
        taskId: task.id,
        input: same(task, !task.enabled),
      });
      await load();
    });
  const summary = (task: ScheduledTask) => {
    if (task.action === "command")
      return t("schedule.commandSummary", {
        command: task.command || "",
      });
    if (task.action === "cleanup")
      return t("schedule.cleanupSummary", { n: task.keepDays });
    return task.mode === "daily"
      ? t("schedule.frequencyDaily", { time: task.atTime || "—" })
      : t("schedule.frequencyInterval", { n: task.everyMinutes });
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
      <div className="info-strip">
        <ShieldCheck size={18} />
        <span>{t("schedule.tip")}</span>
      </div>
      {message && (
        <p className="success-text" role="status">
          {message}
        </p>
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
                      className={
                        task.lastResult === "ok" ? "ok" : "fail"
                      }
                      title={task.lastResult}
                    >
                      {task.lastResult === "ok"
                        ? t("schedule.success")
                        : t("schedule.failed")}
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
              </select>
            </Field>
            {form.value.mode === "daily" ? (
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
            ) : (
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
