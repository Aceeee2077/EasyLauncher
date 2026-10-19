import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Activity,
  FileSearch,
  HeartPulse,
  RefreshCw,
  ShieldCheck,
  Stethoscope,
  TriangleAlert,
} from "lucide-react";
import { Button } from "../components/ui";
import { call } from "../services/api";
import { useI18n, type MessageKey } from "../i18n";
import type {
  DiagnosisReport,
  HealthCheck,
  HealthReport,
  ServerView,
} from "../types";
import type { TaskProps } from "../utils/task";

const scoreClass = (score: number) =>
  score >= 90 ? "good" : score >= 70 ? "fair" : "poor";

export function Ops({ server, run, busy }: TaskProps & { server: ServerView }) {
  const { t } = useI18n();
  const [health, setHealth] = useState<HealthReport | null>(null);
  const [report, setReport] = useState<DiagnosisReport | null>(null);
  const key = `ops-${server.id}`;
  const load = async () => {
    const [nextHealth, nextReport] = await Promise.all([
      call("server_health", { id: server.id }),
      call("server_diagnosis", { id: server.id }),
    ]);
    setHealth(nextHealth);
    setReport(nextReport);
  };
  useEffect(() => {
    void run(`ops-load-${server.id}-${crypto.randomUUID()}`, load);
    const subscription = listen<DiagnosisReport>("server-diagnosis", (event) => {
      if (event.payload.serverId === server.id) setReport(event.payload);
    });
    return () => {
      void subscription.then((stop) => stop());
    };
  }, [server.id]);
  const row = (check: HealthCheck) => (
    <div className={`health-row ${check.level}`} key={check.id}>
      <span className={`health-dot ${check.level}`} />
      <div>
        <strong>
          {t(`health.${check.id}.title` as MessageKey)}
        </strong>
        <p>
          {t(`health.${check.id}.${check.level}` as MessageKey, check.values)}
        </p>
      </div>
      <span className={`tag ${check.level}`}>
        {t(`ops.level.${check.level}` as MessageKey)}
      </span>
    </div>
  );
  const badge = (score: number) =>
    t(
      (score >= 90
        ? "ops.verdict.good"
        : score >= 70
          ? "ops.verdict.fair"
          : "ops.verdict.poor") as MessageKey,
    );
  return (
    <div className="ops-stack">
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <HeartPulse size={18} />
              {t("ops.healthTitle")}
            </h2>
            <p>{t("ops.healthDesc")}</p>
          </div>
          <Button
            busy={busy.has(key)}
            disabled={!health}
            onClick={() => void run(key, load)}
          >
            <RefreshCw size={15} />
            {t("ops.refresh")}
          </Button>
        </div>
        {health ? (
          <>
            <div className="health-summary">
              <div className={`health-score ${scoreClass(health.score)}`}>
                <strong>{health.score}</strong>
                <small>{t("ops.score")}</small>
              </div>
              <div className="health-summary-text">
                <strong>{badge(health.score)}</strong>
                <p>
                  {t("ops.checkedAt", {
                    time: new Date(health.generatedAt).toLocaleTimeString(),
                  })}
                </p>
              </div>
            </div>
            <div className="health-list">{health.checks.map(row)}</div>
          </>
        ) : (
          <p className="mini-empty">{t("common.loading")}</p>
        )}
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Stethoscope size={18} />
              {t("ops.diagnosisTitle")}
            </h2>
            <p>{t("ops.diagnosisDesc")}</p>
          </div>
          <Button
            busy={busy.has(key)}
            disabled={!report}
            onClick={() => void run(key, load)}
          >
            <FileSearch size={15} />
            {t("ops.analyze")}
          </Button>
        </div>
        {report?.source && (
          <p className="diag-source">
            {t("ops.source", { source: report.source })}
          </p>
        )}
        {report && !report.available ? (
          <div className="mini-empty">
            <FileSearch size={36} />
            <h3>{t("ops.noLogsTitle")}</h3>
            <p>{t("ops.noLogsDesc")}</p>
          </div>
        ) : report?.findings.length ? (
          <div className="diag-list">
            {report.findings.map((finding) => (
              <article
                className={`diag-item ${finding.severity}`}
                key={finding.id}
              >
                <div className="diag-head">
                  {finding.severity === "critical" ? (
                    <TriangleAlert size={17} />
                  ) : (
                    <Activity size={17} />
                  )}
                  <strong>
                    {t(`diagnosis.${finding.id}.title` as MessageKey)}
                  </strong>
                  <span className={`tag ${finding.severity}`}>
                    {t(`ops.severity.${finding.severity}` as MessageKey)}
                  </span>
                </div>
                <p>{t(`diagnosis.${finding.id}.advice` as MessageKey)}</p>
                {finding.evidence.length > 0 && (
                  <details className="diag-evidence">
                    <summary>{t("ops.evidence")}</summary>
                    <pre>{finding.evidence.join("\n")}</pre>
                  </details>
                )}
              </article>
            ))}
          </div>
        ) : report ? (
          <div className="mini-empty">
            <ShieldCheck size={36} />
            <h3>{t("ops.cleanTitle")}</h3>
            <p>{t("ops.cleanDesc")}</p>
          </div>
        ) : (
          <p className="mini-empty">{t("common.loading")}</p>
        )}
      </section>
    </div>
  );
}
