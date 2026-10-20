import { useEffect, useState } from "react";
import {
  Cloud,
  CloudUpload,
  KeyRound,
  PlugZap,
  ShieldCheck,
} from "lucide-react";
import { Button, Field, Modal, bytes, date } from "./ui";
import { call } from "../services/api";
import type {
  AppError,
  Backup,
  RemoteArchive,
  RemoteConfig,
  RemoteInfo,
  RemoteSecretsInput,
} from "../types";
import type { TaskProps } from "../utils/task";
import { useI18n } from "../i18n";

const DEFAULT_CONFIG: RemoteConfig = {
  kind: "webdav",
  endpoint: "",
  bucket: "",
  region: "us-east-1",
  prefix: "",
  remote: "",
  username: "",
  accessKey: "",
  rclonePath: "",
  encrypt: true,
  verifyContent: true,
  retries: 3,
  pathStyle: true,
};

type CredentialDraft = {
  password: string;
  secretKey: string;
  passphrase: string;
  clear: boolean;
};

const EMPTY_CREDENTIALS: CredentialDraft = {
  password: "",
  secretKey: "",
  passphrase: "",
  clear: false,
};

export function RemoteBackup({
  id,
  backups,
  run,
  busy,
}: TaskProps & { id: string; backups: Backup[] }) {
  const { t } = useI18n();
  const [info, setInfo] = useState<RemoteInfo | null>(null);
  const [loadError, setLoadError] = useState("");
  const [draft, setDraft] = useState<RemoteConfig | null>(null);
  const [credentials, setCredentials] =
    useState<CredentialDraft>(EMPTY_CREDENTIALS);
  const [selected, setSelected] = useState("");
  const [uploading, setUploading] = useState<Backup | null>(null);
  const [message, setMessage] = useState("");

  const load = async () => {
    try {
      const current = await call("remote_backup_info", { id });
      setInfo(current);
      setLoadError("");
      return current;
    } catch (error) {
      setLoadError((error as AppError).message);
      return null;
    }
  };
  useEffect(() => {
    let alive = true;
    call("remote_backup_info", { id })
      .then((current) => {
        if (!alive) return;
        setInfo(current);
        setLoadError("");
      })
      .catch((error: AppError) => {
        if (alive) setLoadError(error.message);
      });
    return () => {
      alive = false;
    };
  }, [id]);

  const working =
    busy.has("remoteSave") ||
    busy.has("remoteTest") ||
    busy.has("remoteUpload");
  const configured = info?.configured ?? false;
  const config = info?.config ?? DEFAULT_CONFIG;
  const selectedBackup =
    backups.find((backup) => backup.id === selected) ??
    backups.find((backup) => backup.id === info?.pending?.backupId) ??
    backups[0];
  const kindLabel = (kind: RemoteConfig["kind"]) =>
    kind === "s3"
      ? t("backups.remoteKindS3")
      : kind === "rclone"
        ? t("backups.remoteKindRclone")
        : t("backups.remoteKindWebdav");
  const target = (config: RemoteConfig) =>
    config.kind === "webdav"
      ? `${config.endpoint}${config.prefix ? `/${config.prefix}` : ""}`
      : config.kind === "s3"
        ? `${config.bucket} · ${config.region}${config.prefix ? `/${config.prefix}` : ""}`
        : `${config.remote}${config.prefix ? `/${config.prefix}` : ""}`;
  const verificationLabel = (verification: string) =>
    verification === "sha256"
      ? t("backups.remoteVerificationSha")
      : t("backups.remoteVerificationSize");
  const openEditor = () => {
    setCredentials(EMPTY_CREDENTIALS);
    setDraft({ ...(info?.config ?? DEFAULT_CONFIG) });
    setMessage("");
  };
  const save = () =>
    void run("remoteSave", async () => {
      if (!draft) return;
      const secrets: RemoteSecretsInput = {
        password: credentials.password || null,
        secretKey: credentials.secretKey || null,
        passphrase: credentials.passphrase || null,
        clear: credentials.clear,
      };
      const saved = await call("save_remote_backup", {
        id,
        config: draft,
        credentials: secrets,
      });
      setInfo(saved);
      setDraft(null);
      setCredentials(EMPTY_CREDENTIALS);
      setMessage(t("backups.remoteSaved"));
    });
  const test = () =>
    void run("remoteTest", async () => {
      await call("test_remote_backup", { id });
      setMessage(t("backups.remoteTestOk"));
    });
  const upload = () =>
    void run("remoteUpload", async () => {
      if (!uploading) return;
      const outcome = await call("upload_backup_remote", {
        id,
        backupId: uploading.id,
        confirmed: true,
      });
      await load();
      setMessage(
        t("backups.remoteUploaded", {
          verification: verificationLabel(outcome.verification),
        }),
      );
      setUploading(null);
    });
  const updateDraft = (patch: Partial<RemoteConfig>) =>
    setDraft((current) => (current ? { ...current, ...patch } : current));

  return (
    <div className="backup-storage remote-backup">
      <div className="storage-title">
        <div>
          <h3>
            <Cloud size={16} />
            {t("backups.remoteTitle")}
          </h3>
          <p>{t("backups.remoteDesc")}</p>
        </div>
        <div className="row">
          {configured && (
            <Button
              disabled={working}
              busy={busy.has("remoteTest")}
              onClick={test}
            >
              <PlugZap size={15} />
              {t("backups.remoteTest")}
            </Button>
          )}
          <Button variant="primary" disabled={working} onClick={openEditor}>
            <KeyRound size={15} />
            {configured
              ? t("backups.remoteEdit")
              : t("backups.remoteConfigure")}
          </Button>
        </div>
      </div>
      {loadError && <p className="warning-text">{loadError}</p>}
      {info && !info.secretsAvailable && (
        <p className="warning-text">{t("backups.remoteSecretsUnavailable")}</p>
      )}
      {!configured && (
        <div className="info-strip">
          <ShieldCheck size={18} />
          <span>{t("backups.remoteNotConfigured")}</span>
        </div>
      )}
      {info && configured && (
        <>
          <div className="remote-summary">
            <div>
              <small>{t("backups.remoteKind")}</small>
              <strong>{kindLabel(config.kind)}</strong>
            </div>
            <div>
              <small>{t("backups.remoteTarget")}</small>
              <strong className="remote-value">{target(config)}</strong>
            </div>
            <div>
              <small>{t("backups.remoteEncryption")}</small>
              <strong>
                {config.encrypt
                  ? t("backups.remoteEncrypted")
                  : t("backups.remotePlain")}
              </strong>
            </div>
            <div>
              <small>{t("backups.remoteVerification")}</small>
              <strong>
                {config.verifyContent
                  ? t("backups.remoteVerifySha")
                  : t("backups.remoteVerifySize")}
              </strong>
            </div>
          </div>
          {config.kind !== "rclone" && (
            <p className="small muted">
              {info.secretSet
                ? t("backups.remoteCredentialsSaved")
                : t("backups.remoteCredentialsMissing")}
            </p>
          )}
          {config.encrypt && (
            <p className="small muted">
              {info.passphraseSet
                ? t("backups.remotePassphraseSavedShort")
                : t("backups.remotePassphraseMissing")}
            </p>
          )}
          {config.kind !== "rclone" &&
            config.endpoint.startsWith("http://") && (
              <p className="warning-text">
                {t("backups.remoteHttpWarning")}
              </p>
            )}
          {config.kind === "rclone" && !info.rcloneAvailable && (
            <p className="warning-text">{t("backups.remoteRcloneMissing")}</p>
          )}
          {info.pending && (
            <p className="warning-text">
              {t("backups.remotePending", { parts: info.pending.parts })}
            </p>
          )}
          <div className="remote-actions">
            <Field label={t("backups.remoteSelect")}>
              <select
                value={selectedBackup?.id ?? ""}
                disabled={working || !backups.length}
                onChange={(event) => setSelected(event.target.value)}
              >
                {backups.map((backup) => (
                  <option key={backup.id} value={backup.id}>
                    {backup.label || date(backup.createdAt)} ·{" "}
                    {bytes(backup.sizeBytes)}
                    {backup.reason === "safety"
                      ? ` · ${t("backups.safety")}`
                      : ""}
                  </option>
                ))}
              </select>
            </Field>
            <Button
              variant="primary"
              disabled={working || !selectedBackup}
              busy={busy.has("remoteUpload")}
              onClick={() => selectedBackup && setUploading(selectedBackup)}
            >
              <CloudUpload size={16} />
              {t("backups.remoteUpload")}
            </Button>
          </div>
          {message && (
            <p className="success-text" role="status">
              {message}
            </p>
          )}
          <div className="remote-history">
            <div className="remote-history-head">
              <strong>{t("backups.remoteHistory")}</strong>
              <small>
                {t("backups.remoteHistoryCount", {
                  count: info.archives.length,
                })}
              </small>
            </div>
            {info.archives.length ? (
              <div className="remote-archives">
                {info.archives.slice(0, 8).map((archive) => (
                  <RemoteArchiveRow
                    key={archive.object}
                    archive={archive}
                    verification={verificationLabel(archive.verification)}
                  />
                ))}
              </div>
            ) : (
              <p className="small muted">{t("backups.remoteHistoryEmpty")}</p>
            )}
            {info.archives.length > 8 && (
              <p className="small muted">
                {t("backups.remoteHistoryMore", { count: 8 })}
              </p>
            )}
          </div>
          <p className="small muted">{t("backups.remoteUploadNote")}</p>
          <p className="small muted">{t("backups.remoteRestoreNote")}</p>
        </>
      )}
      {draft && (
        <Modal
          title={t("backups.remoteConfigureTitle")}
          onClose={() => {
            if (!busy.has("remoteSave")) setDraft(null);
          }}
        >
          <Field label={t("backups.remoteKind")}>
            <select
              value={draft.kind}
              onChange={(event) =>
                updateDraft({ kind: event.target.value as RemoteConfig["kind"] })
              }
            >
              <option value="webdav">{t("backups.remoteKindWebdav")}</option>
              <option value="s3">{t("backups.remoteKindS3")}</option>
              <option value="rclone">{t("backups.remoteKindRclone")}</option>
            </select>
          </Field>
          {draft.kind === "webdav" && (
            <>
              <Field
                label={t("backups.remoteEndpoint")}
                hint={t("backups.remoteEndpointHint")}
              >
                <input
                  value={draft.endpoint}
                  placeholder="https://dav.example.com/backups"
                  onChange={(event) =>
                    updateDraft({ endpoint: event.target.value })
                  }
                />
              </Field>
              {draft.endpoint.startsWith("http://") && (
                <p className="warning-text">
                  {t("backups.remoteHttpWarning")}
                </p>
              )}
              <div className="form-grid">
                <Field label={t("backups.remoteUsername")}>
                  <input
                    value={draft.username}
                    autoComplete="username"
                    onChange={(event) =>
                      updateDraft({ username: event.target.value })
                    }
                  />
                </Field>
                <Field
                  label={t("backups.remotePassword")}
                  hint={
                    info?.secretSet
                      ? t("backups.remotePasswordSaved")
                      : undefined
                  }
                >
                  <input
                    type="password"
                    value={credentials.password}
                    autoComplete="new-password"
                    onChange={(event) =>
                      setCredentials({
                        ...credentials,
                        password: event.target.value,
                      })
                    }
                  />
                </Field>
              </div>
            </>
          )}
          {draft.kind === "s3" && (
            <>
              <div className="form-grid">
                <Field
                  label={t("backups.remoteS3Endpoint")}
                  hint={t("backups.remoteS3EndpointHint")}
                >
                  <input
                    value={draft.endpoint}
                    placeholder="http://127.0.0.1:9000"
                    onChange={(event) =>
                      updateDraft({ endpoint: event.target.value })
                    }
                  />
                </Field>
                <Field label={t("backups.remoteBucket")}>
                  <input
                    value={draft.bucket}
                    placeholder="mc-backups"
                    onChange={(event) =>
                      updateDraft({ bucket: event.target.value })
                    }
                  />
                </Field>
                <Field label={t("backups.remoteRegion")}>
                  <input
                    value={draft.region}
                    placeholder="us-east-1"
                    onChange={(event) =>
                      updateDraft({ region: event.target.value })
                    }
                  />
                </Field>
                <Field
                  label={t("backups.remoteAccessKey")}
                  hint={t("backups.remoteAccessKeyHint")}
                >
                  <input
                    value={draft.accessKey}
                    autoComplete="off"
                    onChange={(event) =>
                      updateDraft({ accessKey: event.target.value })
                    }
                  />
                </Field>
                <Field
                  label={t("backups.remoteSecretKey")}
                  hint={
                    info?.secretSet
                      ? t("backups.remoteSecretSaved")
                      : undefined
                  }
                >
                  <input
                    type="password"
                    value={credentials.secretKey}
                    autoComplete="new-password"
                    onChange={(event) =>
                      setCredentials({
                        ...credentials,
                        secretKey: event.target.value,
                      })
                    }
                  />
                </Field>
              </div>
              {draft.endpoint.startsWith("http://") && (
                <p className="warning-text">
                  {t("backups.remoteHttpWarning")}
                </p>
              )}
              <label className="check-row">
                <input
                  type="checkbox"
                  checked={draft.pathStyle}
                  onChange={(event) =>
                    updateDraft({ pathStyle: event.target.checked })
                  }
                />
                <span>{t("backups.remotePathStyle")}</span>
              </label>
            </>
          )}
          {draft.kind === "rclone" && (
            <>
              <Field
                label={t("backups.remoteRcloneRemote")}
                hint={t("backups.remoteRcloneRemoteHint")}
              >
                <input
                  value={draft.remote}
                  placeholder="myremote:mc-backups"
                  onChange={(event) =>
                    updateDraft({ remote: event.target.value })
                  }
                />
              </Field>
              <Field
                label={t("backups.remoteRclonePath")}
                hint={t("backups.remoteRclonePathHint")}
              >
                <input
                  value={draft.rclonePath}
                  placeholder="C:\\Program Files\\rclone\\rclone.exe"
                  onChange={(event) =>
                    updateDraft({ rclonePath: event.target.value })
                  }
                />
              </Field>
            </>
          )}
          <Field
            label={t("backups.remotePrefix")}
            hint={t("backups.remotePrefixHint")}
          >
            <input
              value={draft.prefix}
              placeholder="mc/backups"
              onChange={(event) => updateDraft({ prefix: event.target.value })}
            />
          </Field>
          <label className="check-row">
            <input
              type="checkbox"
              checked={draft.encrypt}
              onChange={(event) =>
                updateDraft({ encrypt: event.target.checked })
              }
            />
            <span>
              {t("backups.remoteEncrypt")}
              <small>{t("backups.remoteEncryptHint")}</small>
            </span>
          </label>
          {draft.encrypt && (
            <Field
              label={t("backups.remotePassphrase")}
              hint={
                info?.passphraseSet
                  ? t("backups.remotePassphraseSaved")
                  : t("backups.remotePassphraseHint")
              }
            >
              <input
                type="password"
                value={credentials.passphrase}
                autoComplete="new-password"
                onChange={(event) =>
                  setCredentials({
                    ...credentials,
                    passphrase: event.target.value,
                  })
                }
              />
            </Field>
          )}
          <label className="check-row">
            <input
              type="checkbox"
              checked={draft.verifyContent}
              onChange={(event) =>
                updateDraft({ verifyContent: event.target.checked })
              }
            />
            <span>
              {t("backups.remoteVerifyContent")}
              <small>{t("backups.remoteVerifyHint")}</small>
            </span>
          </label>
          <Field label={t("backups.remoteRetries")}>
            <input
              type="number"
              min={0}
              max={10}
              value={draft.retries}
              onChange={(event) =>
                updateDraft({
                  retries: Math.max(
                    0,
                    Math.min(10, Number(event.target.value) || 0),
                  ),
                })
              }
            />
          </Field>
          <label className="check-row">
            <input
              type="checkbox"
              checked={credentials.clear}
              onChange={(event) =>
                setCredentials({ ...credentials, clear: event.target.checked })
              }
            />
            <span>{t("backups.remoteClear")}</span>
          </label>
          <p className="small muted">{t("backups.remoteCredentialNote")}</p>
          <div className="modal-actions">
            <Button
              disabled={busy.has("remoteSave")}
              onClick={() => setDraft(null)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              busy={busy.has("remoteSave")}
              onClick={save}
            >
              {t("backups.remoteSave")}
            </Button>
          </div>
        </Modal>
      )}
      {uploading && (
        <Modal
          title={t("backups.remoteUploadTitle")}
          onClose={() => {
            if (!busy.has("remoteUpload")) setUploading(null);
          }}
        >
          <p>
            {t("backups.remoteUploadBody", {
              date: date(uploading.createdAt),
              target: target(config),
            })}
          </p>
          <p className="small muted">
            {config.encrypt
              ? t("backups.remoteEncrypted")
              : t("backups.remotePlain")}
            {" · "}
            {config.verifyContent
              ? t("backups.remoteVerifySha")
              : t("backups.remoteVerifySize")}
          </p>
          <p className="small muted">{t("backups.remoteUploadNote")}</p>
          <div className="modal-actions">
            <Button
              disabled={busy.has("remoteUpload")}
              onClick={() => setUploading(null)}
            >
              {t("common.cancel")}
            </Button>
            <Button
              variant="primary"
              busy={busy.has("remoteUpload")}
              onClick={upload}
            >
              <CloudUpload size={16} />
              {t("backups.remoteUploadConfirm")}
            </Button>
          </div>
        </Modal>
      )}
    </div>
  );
}

function RemoteArchiveRow({
  archive,
  verification,
}: {
  archive: RemoteArchive;
  verification: string;
}) {
  const { t } = useI18n();
  return (
    <div>
      <span>
        <strong>{archive.label || date(archive.backupCreatedAt)}</strong>
        <small>{archive.object}</small>
      </span>
      <span className="tag">
        {archive.encrypted ? t("backups.remoteEncryptedShort") : "ZIP"}
      </span>
      <span>
        {bytes(archive.bytes)}
        <small>{date(archive.uploadedAt)}</small>
      </span>
      <span className="tag">{verification}</span>
    </div>
  );
}
