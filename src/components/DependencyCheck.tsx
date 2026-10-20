import { useEffect, useState } from "react";
import { GitBranch, RefreshCw } from "lucide-react";
import { Button } from "./ui";
import { call } from "../services/api";
import { useI18n, type MessageKey } from "../i18n";
import type { DependencyReport } from "../types";
import type { TaskProps } from "../utils/task";

export function DependencyCheck({ id, run, busy }: TaskProps & { id: string }) {
  const { t } = useI18n();
  const [report, setReport] = useState<DependencyReport | null>(null);
  const key = `dependencies-${id}`;
  useEffect(() => {
    let alive = true;
    void run(`${key}-load-${crypto.randomUUID()}`, async () => {
      const result = await call("check_dependencies", { id, remote: false });
      if (alive) setReport(result);
    });
    return () => {
      alive = false;
    };
  }, [id]);
  return (
    <section className="panel">
      <div className="section-title">
        <div>
          <h2>
            <GitBranch size={18} />
            {t("deps.title")}
          </h2>
          <p>{t("deps.desc")}</p>
        </div>
        <Button
          busy={busy.has(key)}
          onClick={() =>
            void run(key, async () =>
              setReport(await call("check_dependencies", { id, remote: true })),
            )
          }
        >
          <RefreshCw size={15} />
          {t("deps.check")}
        </Button>
      </div>
      <p className="small muted">{t("deps.limit")}</p>
      {report ? (
        <>
          <p className="info-strip">
            {t("deps.summary", {
              modules: report.modules.length,
              edges: report.edges.length,
              issues: report.issues.length,
            })}{" "}
            · {t(report.remoteChecked ? "deps.online" : "deps.local")}
          </p>
          {report.issues.length ? (
            <div className="diag-list">
              {report.issues.map((issue, i) => (
                <article
                  className="diag-item warning"
                  key={`${issue.source}-${issue.target}-${i}`}
                >
                  <strong>{t(`deps.${issue.code}` as MessageKey)}</strong>
                  <p>
                    {issue.source}
                    {issue.target ? ` → ${issue.target}` : ""}
                  </p>
                  {issue.detail && (
                    <code className="break-anywhere">{issue.detail}</code>
                  )}
                </article>
              ))}
            </div>
          ) : (
            <p className="success-text">{t("deps.clean")}</p>
          )}
          <details>
            <summary>{t("deps.graph")}</summary>
            <div className="dependency-graph">
              {report.edges.slice(0, 300).map((edge, i) => (
                <div
                  className="dependency-edge"
                  key={`${edge.source}-${edge.target}-${i}`}
                >
                  <code>{edge.source}</code>
                  <span>
                    → {t(`deps.relation.${edge.relation}` as MessageKey)} →
                  </span>
                  <code>{edge.target}</code>
                  <small>{edge.requirement}</small>
                </div>
              ))}
              {report.edges.length > 300 && <p>{t("deps.truncated")}</p>}
            </div>
          </details>
          <details>
            <summary>{t("deps.modules")}</summary>
            <div className="content-rows">
              {report.modules.map((m, i) => (
                <div className="content-row" key={`${m.file}-${i}`}>
                  <div>
                    <strong>
                      {m.id} · {m.version}
                    </strong>
                    <small className="break-anywhere">
                      {t(`deps.kind.${m.kind}` as MessageKey)} · {m.file}
                    </small>
                  </div>
                </div>
              ))}
            </div>
          </details>
        </>
      ) : (
        <p>{t("common.loading")}</p>
      )}
    </section>
  );
}
