import { useEffect, useState } from "react";
import { Archive, RotateCcw, Trash2, Plus, ShieldCheck } from "lucide-react";
import { Button, Modal, bytes, date } from "../components/ui";
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
  const [message, setMessage] = useState("");
  const load = async () =>
    setBackups(await call("list_backups", { id: server.id }));
  useEffect(() => {
    void run(`backups-${server.id}-${crypto.randomUUID()}`, load);
  }, [server.id]);
  const available = idle(server.runtime.status);
  const working = busy.has("backup") || busy.has("backupAction");
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
                <strong>{date(b.createdAt)}</strong>
                <small>{b.worlds.join(", ")}</small>
              </span>
              <span>{bytes(b.sizeBytes)}</span>
              <span className="tag">
                {b.safety ? t("backups.safety") : t("backups.manual")}
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
              onClick={() =>
                void run("backupAction", async () => {
                  const args = {
                    id: server.id,
                    backupId: confirm.b.id,
                    confirmed: true,
                  };
                  await (confirm.action === "restore"
                    ? call("restore_backup", args)
                    : call("delete_backup", args));
                  setMessage(
                    confirm.action === "restore"
                      ? t("backups.restored")
                      : t("backups.deleted"),
                  );
                  setConfirm(null);
                  await load();
                })
              }
            >
              {confirm.action === "restore"
                ? t("backups.confirmRestore")
                : t("backups.confirmDelete")}
            </Button>
          </div>
        </Modal>
      )}
    </section>
  );
}
