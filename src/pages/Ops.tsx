import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  Activity,
  Check,
  Copy,
  FileSearch,
  FileText,
  HeartPulse,
  RefreshCw,
  ShieldCheck,
  Stethoscope,
  TriangleAlert,
} from "lucide-react";
import { Button } from "../components/ui";
import { LogAssistant } from "../components/LogAssistant";
import { DiskUsage } from "../components/DiskUsage";
import { ProductionService } from "../components/ProductionService";
import { ActivityWorkbench } from "../components/ActivityWorkbench";
import { call } from "../services/api";
import { useI18n, type MessageKey } from "../i18n";
import type {
  CrashReport,
  DiagnosisReport,
  HealthCheck,
  HealthReport,
  ServerView,
} from "../types";
import type { TaskProps } from "../utils/task";

const scoreClass = (score: number) =>
  score >= 90 ? "good" : score >= 70 ? "fair" : "poor";

export function Ops({ server, run, busy }: TaskProps & { server: ServerView }) {
  const { t, locale } = useI18n();
  const [health, setHealth] = useState<HealthReport | null>(null);
  const [report, setReport] = useState<DiagnosisReport | null>(null);
  const [crashReport, setCrashReport] = useState<CrashReport | null>(null);
  const [copyState, setCopyState] = useState<"idle" | "copied" | "failed">(
    "idle",
  );
  const key = `ops-${server.id}`;
  const reportKey = `crash-report-${server.id}`;
  const load = async () => {
    const [nextHealth, nextReport] = await Promise.all([
      call("server_health", { id: server.id }),
      call("server_diagnosis", { id: server.id }),
    ]);
    setHealth(nextHealth);
    setReport(nextReport);
    setCrashReport(null);
    setCopyState("idle");
  };
  const generateReport = async () => {
    const next = await call("server_crash_report", {
      id: server.id,
      language: locale,
    });
    setCrashReport(next);
    setCopyState("idle");
  };
  const copyReport = async () => {
    if (!crashReport) return;
    try {
      await navigator.clipboard.writeText(crashReport.text);
      setCopyState("copied");
    } catch {
      setCopyState("failed");
    }
  };
  useEffect(() => {
    void run(`ops-load-${server.id}-${crypto.randomUUID()}`, load);
    const subscription = listen<DiagnosisReport>(
      "server-diagnosis",
      (event) => {
        if (event.payload.serverId === server.id) setReport(event.payload);
      },
    );
    return () => {
      void subscription.then((stop) => stop());
    };
  }, [server.id]);
  const row = (check: HealthCheck) => (
    <div className={`health-row ${check.level}`} key={check.id}>
      <span className={`health-dot ${check.level}`} />
      <div>
        <strong>{t(`health.${check.id}.title` as MessageKey)}</strong>
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
  const checks = health?.checks ?? [];
  const issues = checks.filter(
    (check) => check.level === "critical" || check.level === "warning",
  );
  const rest = checks.filter(
    (check) => check.level === "ok" || check.level === "info",
  );
  return (
    <div className="ops-stack">
      <ActivityWorkbench key={server.id} id={server.id} run={run} busy={busy} />
      <LogAssistant id={server.id} run={run} busy={busy} />
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
                <p className="health-counts">
                  {t("ops.summary", {
                    passed: checks.filter((check) => check.level === "ok").length,
                    warnings: issues.filter((check) => check.level === "warning")
                      .length,
                    critical: issues.filter(
                      (check) => check.level === "critical",
                    ).length,
                  })}
                </p>
              </div>
            </div>
            {issues.length > 0 ? (
              <>
                <div className="health-list">{issues.map(row)}</div>
                <details className="health-passed">
                  <summary>
                    {t("ops.passed", { count: rest.length })}
                  </summary>
                  <div className="health-list">{rest.map(row)}</div>
                </details>
              </>
            ) : (
              <div className="health-list">{checks.map(row)}</div>
            )}
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
          <div className="diag-actions">
            <Button
              variant="ghost"
              busy={busy.has(reportKey)}
              disabled={!report?.findings.length}
              onClick={() => void run(reportKey, generateReport)}
            >
              <FileText size={15} />
              {t("ops.report")}
            </Button>
            <Button
              busy={busy.has(key)}
              disabled={!report}
              onClick={() => void run(key, load)}
            >
              <FileSearch size={15} />
              {t("ops.analyze")}
            </Button>
          </div>
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
                className={`diag-item ${finding.severity}${finding.rootCause ? " root" : ""}`}
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
                  {finding.rootCause && (
                    <span className="tag root">{t("ops.rootCause")}</span>
                  )}
                  {finding.symptom && (
                    <span className="tag info">{t("ops.symptom")}</span>
                  )}
                  <span className={`tag ${finding.severity}`}>
                    {t(`ops.severity.${finding.severity}` as MessageKey)}
                  </span>
                </div>
                <p>{t(`diagnosis.${finding.id}.advice` as MessageKey)}</p>
                <div className="diag-meta">
                  {finding.line !== null && (
                    <span>
                      {t("ops.firstLine", { line: finding.line })}
                    </span>
                  )}
                  {finding.occurrences > 1 && (
                    <span>
                      {t("ops.occurrences", { count: finding.occurrences })}
                    </span>
                  )}
                  {finding.rootCause && report.exitCode !== null && (
                    <span>{t("ops.exitCode", { code: report.exitCode })}</span>
                  )}
                </div>
                {Object.keys(finding.values).length > 0 && (
                  <div className="diag-values">
                    {Object.entries(finding.values).map(([name, value]) => {
                      const label = `ops.diagValue.${name}` as MessageKey;
                      const text = t(label, { value });
                      return (
                        <span key={name}>
                          {text === label ? `${name}: ${value}` : text}
                        </span>
                      );
                    })}
                  </div>
                )}
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
        {crashReport && (
          <div className="diag-report">
            <div className="diag-report-head">
              <div>
                <strong>{t("ops.reportTitle")}</strong>
                <p>{t("ops.reportDesc")}</p>
              </div>
              <Button variant="ghost" onClick={() => void copyReport()}>
                {copyState === "copied" ? (
                  <Check size={15} />
                ) : (
                  <Copy size={15} />
                )}
                {copyState === "copied"
                  ? t("ops.reportCopied")
                  : t("ops.reportCopy")}
              </Button>
            </div>
            <pre className="diag-report-text">{crashReport.text}</pre>
            <small>
              {t("ops.reportSource", {
                source: crashReport.source ?? "—",
              })}
            </small>
            {copyState === "failed" && (
              <p className="diag-copy-failed">{t("ops.reportCopyFailed")}</p>
            )}
          </div>
        )}
      </section>
      <DiskUsage id={server.id} run={run} busy={busy} />
      <ProductionService id={server.id} run={run} busy={busy} />
    </div>
  );
}
