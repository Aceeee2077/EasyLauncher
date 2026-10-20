import { useEffect, useState } from "react";
import {
  Archive,
  RotateCcw,
  Trash2,
  Plus,
  ShieldCheck,
  History,
  Pencil,
  Undo2,
} from "lucide-react";
import { Button, Field, Modal, bytes, date } from "../components/ui";
import { BackupComparison } from "../components/BackupComparison";
import { call } from "../services/api";
import { idle, type Backup, type ServerView } from "../types";
import type { TaskProps } from "../utils/task";
import { useI18n } from "../i18n";
export function Backups({
  server,
  run,
  busy,
}: TaskProps & { server: ServerView }) {
  const { t } = useI18n();
  const [backups, setBackups] = useState<Backup[]>([]);
  const [confirm, setConfirm] = useState<{
    b: Backup;
    action: "restore" | "delete";
  } | null>(null);
  const [undo, setUndo] = useState<{
    safetyId: string;
    snapshotDate: string;
  } | null>(null);
  const [label, setLabel] = useState<{ b: Backup; value: string } | null>(null);
  const [message, setMessage] = useState("");
  const load = async () =>
    setBackups(await call("list_backups", { id: server.id }));
  useEffect(() => {
    void run(`backups-${server.id}-${crypto.randomUUID()}`, load);
  }, [server.id]);
  const available = idle(server.runtime.status);
  const working = busy.has("backup") || busy.has("backupAction");
  const latest = backups.find((b) => b.reason !== "safety") ?? backups[0];
  const reasonLabel = (b: Backup) =>
    b.reason === "safety"
      ? t("backups.safety")
      : b.reason === "scheduled"
        ? t("backups.scheduled")
        : t("backups.manual");
  const apply = (backupId: string, snapshotDate: string, undoable: boolean) =>
    void run("backupAction", async () => {
      const outcome = await call("restore_backup", {
        id: server.id,
        backupId,
        confirmed: true,
      });
      setMessage(
        undoable
          ? t("backups.rollbackDone", { date: snapshotDate })
          : t("backups.undone"),
      );
      setUndo(
        undoable && outcome.safetyBackupId
          ? { safetyId: outcome.safetyBackupId, snapshotDate }
          : null,
      );
      setConfirm(null);
      await load();
    });
  return (
    <section className="panel">
      <div className="section-title">
        <div>
          <h2>
            <Archive size={18} />
            {t("backups.title")}
          </h2>
          <p>{t("backups.desc")}</p>
        </div>
        <Button
          variant="primary"
          disabled={!available || working}
          busy={busy.has("backup")}
          onClick={() =>
            void run("backup", async () => {
              await call("create_backup", { id: server.id });
              await load();
              setMessage(t("backups.created"));
            })
          }
        >
          <Plus size={16} />
          {t("backups.create")}
        </Button>
      </div>
      <div className="info-strip">
        <ShieldCheck size={18} />
        <span>{t("backups.tip")}</span>
      </div>
      <div className="rollback-card">
        <div className="rollback-icon">
          <History size={20} />
        </div>
        <div className="rollback-copy">
          <small>{t("backups.latestPoint")}</small>
          {latest ? (
            <>
              <strong>{latest.label || date(latest.createdAt)}</strong>
              <span>
                {reasonLabel(latest)} · {bytes(latest.sizeBytes)}
                {latest.minecraftVersion ? ` · ${latest.minecraftVersion}` : ""}
              </span>
            </>
          ) : (
            <strong>{t("backups.rollbackEmpty")}</strong>
          )}
          {latest && <p>{t("backups.rollbackDesc")}</p>}
        </div>
        <Button
          variant="primary"
          disabled={!latest || !available || working}
          onClick={() => latest && setConfirm({ b: latest, action: "restore" })}
        >
          <RotateCcw size={16} />
          {t("backups.rollbackAction")}
        </Button>
      </div>
      {undo && (
        <div className="rollback-banner">
          <div>
            <strong>
              {t("backups.rollbackDone", { date: undo.snapshotDate })}
            </strong>
            <small>{t("backups.undoHint")}</small>
          </div>
          <Button
            disabled={!available || working}
            busy={busy.has("backupAction")}
            onClick={() => apply(undo.safetyId, undo.snapshotDate, false)}
          >
            <Undo2 size={15} />
            {t("backups.undo")}
          </Button>
        </div>
      )}
      {message && (
        <p className="success-text" role="status">
          {message}
        </p>
      )}
      {backups.length ? (
        <div className="backup-table">
          <div className="backup-head">
            <span>{t("backups.createdAt")}</span>
            <span>{t("backups.size")}</span>
            <span>{t("backups.type")}</span>
            <span>{t("backups.actions")}</span>
          </div>
          {backups.map((b) => (
            <div className="backup-row" key={b.id}>
              <span>
                <strong>{b.label || date(b.createdAt)}</strong>
                <small>
                  {b.label ? `${date(b.createdAt)} · ` : ""}
                  {b.worlds.join(", ")}
                </small>
              </span>
              <span>{bytes(b.sizeBytes)}</span>
              <span>
                <span className="tag">{reasonLabel(b)}</span>
                {b.minecraftVersion && <small>{b.minecraftVersion}</small>}
              </span>
              <div className="row">
                <Button
                  disabled={!available || working}
                  onClick={() => setConfirm({ b, action: "restore" })}
                >
                  <RotateCcw size={15} />
                  {t("backups.restore")}
                </Button>
                <Button
                  disabled={working}
                  variant="ghost"
                  aria-label={t("backups.editLabel")}
                  onClick={() => setLabel({ b, value: b.label || "" })}
                >
                  <Pencil size={15} />
                </Button>
                <Button
                  disabled={!available || working}
                  variant="ghost"
                  aria-label={t("backups.delete")}
                  onClick={() => setConfirm({ b, action: "delete" })}
                >
                  <Trash2 size={16} />
                </Button>
              </div>
            </div>
          ))}
        </div>
      ) : (
        <div className="mini-empty">
          <Archive size={36} />
          <h3>{t("backups.emptyTitle")}</h3>
          <p>{t("backups.emptyDesc")}</p>
        </div>
      )}
      <BackupComparison
        key={backups.map((b) => b.id).join(",")}
        id={server.id}
        backups={backups}
        run={run}
        busy={busy}
      />
      {confirm && (
        <Modal
          title={
            confirm.action === "restore"
              ? t("backups.restoreTitle")
              : t("backups.deleteTitle")
          }
          onClose={() => {
            if (!working) setConfirm(null);
          }}
        >
          <p>
            {confirm.action === "restore"
              ? t("backups.restoreBody", { date: date(confirm.b.createdAt) })
              : t("backups.deleteBody")}
          </p>
          <div className="modal-actions">
            <Button onClick={() => setConfirm(null)} disabled={working}>
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              busy={working}
              onClick={() => {
                if (confirm.action === "restore") {
                  apply(confirm.b.id, date(confirm.b.createdAt), true);
                  return;
                }
                void run("backupAction", async () => {
                  await call("delete_backup", {
                    id: server.id,
                    backupId: confirm.b.id,
                    confirmed: true,
                  });
                  setMessage(t("backups.deleted"));
                  setConfirm(null);
                  await load();
                });
              }}
            >
              {confirm.action === "restore"
                ? t("backups.confirmRestore")
                : t("backups.confirmDelete")}
            </Button>
          </div>
        </Modal>
      )}
      {label && (
        <Modal title={t("backups.labelTitle")} onClose={() => setLabel(null)}>
          <Field label={t("backups.labelTitle")} hint={t("backups.labelHint")}>
            <input
              value={label.value}
              maxLength={60}
              placeholder={t("backups.labelPlaceholder")}
              onChange={(e) => setLabel({ ...label, value: e.target.value })}
              autoFocus
            />
          </Field>
          <div className="modal-actions">
            <Button
              onClick={() => setLabel(null)}
              disabled={busy.has("backupLabel")}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              busy={busy.has("backupLabel")}
              onClick={() =>
                void run("backupLabel", async () => {
                  await call("set_backup_label", {
                    id: server.id,
                    backupId: label.b.id,
                    label: label.value.trim() || null,
                  });
                  setMessage(t("backups.labelSaved"));
                  setLabel(null);
                  await load();
                })
              }
            >
              {t("backups.saveLabel")}
            </Button>
          </div>
        </Modal>
      )}
    </section>
  );
}
