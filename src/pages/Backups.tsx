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
  Zap,
  HardDrive,
  Eraser,
} from "lucide-react";
import { Button, Field, Modal, bytes, date } from "../components/ui";
import { BackupComparison } from "../components/BackupComparison";
import { RemoteBackup } from "../components/RemoteBackup";
import { call } from "../services/api";
import {
  idle,
  type Backup,
  type BackupStoreInfo,
  type RconInfo,
  type RetentionPolicy,
  type RetentionPreview,
  type ServerView,
} from "../types";
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
  const [rcon, setRcon] = useState<RconInfo | null>(null);
  const [store, setStore] = useState<BackupStoreInfo | null>(null);
  const [policy, setPolicy] = useState<RetentionPolicy | null>(null);
  const [draft, setDraft] = useState<RetentionPolicy | null>(null);
  const [cleanup, setCleanup] = useState<RetentionPreview | null>(null);
  const load = async () => {
    const [list, info, retention] = await Promise.all([
      call("list_backups", { id: server.id }),
      call("backup_store_info", { id: server.id }),
      call("backup_retention", { id: server.id }),
    ]);
    setBackups(list);
    setStore(info);
    setPolicy(retention);
  };
  useEffect(() => {
    void run(`backups-${server.id}-${crypto.randomUUID()}`, load);
  }, [server.id]);
  const live = server.runtime.status === "running";
  useEffect(() => {
    if (!live) {
      setRcon(null);
      return;
    }
    let alive = true;
    call("rcon_info", { id: server.id })
      .then((info) => {
        if (alive) setRcon(info);
      })
      .catch(() => {
        if (alive) setRcon(null);
      });
    return () => {
      alive = false;
    };
  }, [server.id, live]);
  const available = idle(server.runtime.status);
  const working =
    busy.has("backup") ||
    busy.has("backupAction") ||
    busy.has("liveBackup") ||
    busy.has("retentionSave") ||
    busy.has("retentionPreview") ||
    busy.has("retentionApply");
  const setNumber = (key: "daily" | "weekly" | "monthly", value: string) =>
    setDraft((current) =>
      current
        ? {
            ...current,
            [key]: Math.max(0, Math.min(365, Number(value) || 0)),
          }
        : current,
    );
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
        <div className="row">
          {live && (
            <Button
              variant="primary"
              disabled={!rcon?.enabled || working}
              busy={busy.has("liveBackup")}
              title={
                rcon && !rcon.enabled ? t("backups.liveRconHint") : undefined
              }
              onClick={() =>
                void run("liveBackup", async () => {
                  try {
                    await call("create_live_backup", { id: server.id });
                    setMessage(t("backups.liveCreated"));
                  } finally {
                    await load().catch(() => undefined);
                  }
                })
              }
            >
              <Zap size={16} />
              {t("backups.liveCreate")}
            </Button>
          )}
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
      </div>
      <div className="info-strip">
        <ShieldCheck size={18} />
        <span>{t("backups.tip")}</span>
      </div>
      {store && policy && (
        <div className="backup-storage">
          <div className="storage-title">
            <div>
              <h3>
                <HardDrive size={16} />
                {t("backups.storageTitle")}
              </h3>
              <p>{t("backups.storageDesc")}</p>
            </div>
            <span className="tag">zstd</span>
          </div>
          <div className="storage-stats">
            <div>
              <small>{t("backups.storageBackups")}</small>
              <strong>{store.backups}</strong>
            </div>
            <div>
              <small>{t("backups.storageLogical")}</small>
              <strong>{bytes(store.logicalBytes)}</strong>
            </div>
            <div>
              <small>{t("backups.storageDisk")}</small>
              <strong>{bytes(store.storedBytes)}</strong>
            </div>
            <div>
              <small>{t("backups.storageSaved")}</small>
              <strong>{bytes(store.dedupeSavedBytes)}</strong>
            </div>
          </div>
          {store.legacyBackups > 0 && (
            <p className="small muted">
              {t("backups.storageLegacy", { count: store.legacyBackups })}
            </p>
          )}
          <div className="retention-bar">
            <div>
              <strong>{t("backups.retentionTitle")}</strong>
              <small>
                {policy.enabled
                  ? t("backups.retentionSummary", {
                      daily: policy.daily,
                      weekly: policy.weekly,
                      monthly: policy.monthly,
                    })
                  : t("backups.retentionOff")}
              </small>
            </div>
            <div className="row">
              <Button
                variant="ghost"
                disabled={working}
                onClick={() => setDraft({ ...policy })}
              >
                {t("backups.retentionEdit")}
              </Button>
              <Button
                disabled={!policy.enabled || working || !!draft}
                busy={busy.has("retentionPreview")}
                onClick={() =>
                  void run("retentionPreview", async () => {
                    const result = await call("preview_backup_cleanup", {
                      id: server.id,
                    });
                    setCleanup(result);
                  })
                }
              >
                <Eraser size={15} />
                {t("backups.retentionPreview")}
              </Button>
            </div>
          </div>
          {draft && (
            <div className="retention-editor">
              <label className="check-row">
                <input
                  type="checkbox"
                  checked={draft.enabled}
                  onChange={(e) =>
                    setDraft({ ...draft, enabled: e.target.checked })
                  }
                />
                <span>{t("backups.retentionEnabled")}</span>
              </label>
              <div className="retention-grid">
                <Field label={t("backups.retentionDaily")}>
                  <input
                    type="number"
                    min={0}
                    max={365}
                    value={draft.daily}
                    onChange={(e) => setNumber("daily", e.target.value)}
                  />
                </Field>
                <Field label={t("backups.retentionWeekly")}>
                  <input
                    type="number"
                    min={0}
                    max={365}
                    value={draft.weekly}
                    onChange={(e) => setNumber("weekly", e.target.value)}
                  />
                </Field>
                <Field label={t("backups.retentionMonthly")}>
                  <input
                    type="number"
                    min={0}
                    max={365}
                    value={draft.monthly}
                    onChange={(e) => setNumber("monthly", e.target.value)}
                  />
                </Field>
              </div>
              <label className="check-row">
                <input
                  type="checkbox"
                  checked={draft.keepLabeled}
                  onChange={(e) =>
                    setDraft({ ...draft, keepLabeled: e.target.checked })
                  }
                />
                <span>{t("backups.retentionKeepLabeled")}</span>
              </label>
              <p className="small muted">{t("backups.retentionHint")}</p>
              <div className="modal-actions">
                <Button
                  disabled={busy.has("retentionSave")}
                  onClick={() => setDraft(null)}
                >
                  {t("common.cancel")}
                </Button>
                <Button
                  variant="primary"
                  busy={busy.has("retentionSave")}
                  onClick={() =>
                    void run("retentionSave", async () => {
                      const saved = await call("set_backup_retention", {
                        id: server.id,
                        policy: draft,
                      });
                      setPolicy(saved);
                      setDraft(null);
                      setMessage(t("backups.retentionSaved"));
                    })
                  }
                >
                  {t("backups.retentionSave")}
                </Button>
              </div>
            </div>
          )}
        </div>
      )}
      <RemoteBackup
        id={server.id}
        backups={backups}
        run={run}
        busy={busy}
      />
      {live && rcon && !rcon.enabled && (
        <p className="small muted">{t("backups.liveRconHint")}</p>
      )}
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
              <span>
                {bytes(b.sizeBytes)}
                {b.format === "chunked" && (
                  <small>
                    {t("backups.compressedSize", {
                      size: bytes(b.storedBytes),
                    })}
                  </small>
                )}
              </span>
              <span>
                <span className="tag">{reasonLabel(b)}</span>
                {b.format === "chunked" && <small>zstd</small>}
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
      {cleanup && (
        <Modal
          title={t("backups.retentionPreviewTitle")}
          onClose={() => {
            if (!busy.has("retentionApply")) setCleanup(null);
          }}
        >
          <p>
            {t("backups.retentionPreviewBody", {
              count: cleanup.candidates.length,
              keep: cleanup.keep,
              size: bytes(cleanup.reclaimableBytes),
            })}
          </p>
          {cleanup.candidates.length ? (
            <div className="retention-candidates">
              {cleanup.candidates.map((item) => (
                <div key={item.id}>
                  <span>{item.label || date(item.createdAt)}</span>
                  <span className="tag">{reasonLabel(item)}</span>
                  <span>{bytes(item.sizeBytes)}</span>
                </div>
              ))}
            </div>
          ) : (
            <div className="info-strip">
              <ShieldCheck size={18} />
              <span>{t("backups.retentionNothing")}</span>
            </div>
          )}
          <div className="modal-actions">
            <Button
              disabled={busy.has("retentionApply")}
              onClick={() => setCleanup(null)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="danger"
              busy={busy.has("retentionApply")}
              disabled={!cleanup.candidates.length}
              onClick={() =>
                void run("retentionApply", async () => {
                  const outcome = await call("apply_backup_cleanup", {
                    id: server.id,
                    confirmed: true,
                  });
                  setMessage(
                    t("backups.retentionApplied", {
                      count: outcome.deleted,
                      size: bytes(outcome.reclaimedBytes),
                    }),
                  );
                  setCleanup(null);
                  await load();
                })
              }
            >
              <Eraser size={15} />
              {t("backups.retentionApply")}
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
