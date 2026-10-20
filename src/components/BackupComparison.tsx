import { useState } from "react";
import { Folder, GitCompareArrows } from "lucide-react";
import { Button, Field, bytes, date } from "./ui";
import { call } from "../services/api";
import { useI18n, type MessageKey } from "../i18n";
import type { Backup, BackupDiff } from "../types";
import type { TaskProps } from "../utils/task";

export function BackupComparison({
  id,
  backups,
  run,
  busy,
}: TaskProps & { id: string; backups: Backup[] }) {
  const { t } = useI18n();
  const [before, setBefore] = useState("");
  const [after, setAfter] = useState("");
  const [diff, setDiff] = useState<BackupDiff | null>(null);
  const key = `backup-diff-${id}`;
  const beforeId = backups.some((b) => b.id === before)
    ? before
    : (backups[1]?.id ?? "");
  const afterId = backups.some((b) => b.id === after)
    ? after
    : (backups[0]?.id ?? "");
  const folders = new Map<string, BackupDiff["changes"]>();
  for (const file of diff?.changes ?? []) {
    const name = file.path.slice(0, file.path.lastIndexOf("/")) || "/";
    const entries = folders.get(name) ?? [];
    entries.push(file);
    folders.set(name, entries);
  }
  return (
    <div className="backup-comparison">
      <h3>
        <GitCompareArrows size={18} />
        {t("diff.title")}
      </h3>
      <p className="small muted">{t("diff.desc")}</p>
      <div className="compare-controls">
        {(["before", "after"] as const).map((side) => (
          <Field key={side} label={t(`diff.${side}`)}>
            <select
              value={side === "before" ? beforeId : afterId}
              disabled={busy.has(key)}
              onChange={(e) => {
                (side === "before" ? setBefore : setAfter)(e.target.value);
                setDiff(null);
              }}
            >
              {!backups.length && <option value="">—</option>}
              {backups.map((b) => (
                <option value={b.id} key={b.id}>
                  {b.label || date(b.createdAt)} · {b.id.slice(0, 8)}
                </option>
              ))}
            </select>
          </Field>
        ))}
        <Button
          busy={busy.has(key)}
          disabled={!beforeId || !afterId || beforeId === afterId}
          onClick={() =>
            void run(key, async () => {
              setDiff(null);
              setDiff(await call("compare_backups", { id, beforeId, afterId }));
            })
          }
        >
          {t("diff.compare")}
        </Button>
      </div>
      {diff && (
        <>
          <div className="diff-summary" role="status">
            {(["added", "removed", "modified", "unchanged"] as const).map(
              (kind) => (
                <span key={kind} className={`tag ${kind}`}>
                  {t(`diff.${kind}`)} {diff[kind]}
                </span>
              ),
            )}
            <span>
              {bytes(diff.beforeBytes)} → {bytes(diff.afterBytes)}
            </span>
          </div>
          <div className="diff-tree">
            {Array.from(folders, ([name, files]) => (
              <details key={name} open>
                <summary>
                  <Folder size={14} /> {name} ({files.length})
                </summary>
                {files.map((file) => (
                  <div className={`diff-file ${file.kind}`} key={file.path}>
                    <span>{t(`diff.${file.kind}` as MessageKey)}</span>
                    <code title={file.path}>
                      {file.path.slice(file.path.lastIndexOf("/") + 1)}
                    </code>
                    <small>
                      {bytes(file.beforeBytes)} → {bytes(file.afterBytes)}
                    </small>
                  </div>
                ))}
              </details>
            ))}
          </div>
          {!diff.changes.length && (
            <p className="success-text">{t("diff.identical")}</p>
          )}
          {diff.truncated && (
            <p className="small muted">{t("diff.truncated")}</p>
          )}
        </>
      )}
    </div>
  );
}
