import { useEffect, useState } from "react";
import {
  Copy,
  Globe,
  RefreshCw,
  Router,
  ShieldCheck,
  TriangleAlert,
  Wifi,
} from "lucide-react";
import { Button, Field, date } from "../components/ui";
import { call, desktop } from "../services/api";
import { useI18n } from "../i18n";
import { active } from "../types";
import type { NetworkCheck, NetworkReport, ServerView } from "../types";
import type { TaskProps } from "../utils/task";

export function NetworkPanel({
  server,
  run,
  busy,
}: TaskProps & { server: ServerView }) {
  const { t } = useI18n();
  const [report, setReport] = useState<NetworkReport | null>(null);
  const [check, setCheck] = useState<NetworkCheck | null>(null);
  const [port, setPort] = useState(String(server.port));
  const [message, setMessage] = useState("");
  const [allowMessage, setAllowMessage] = useState("");
  const key = `network-${server.id}`;
  const mapped = Boolean(report?.mapping?.enabled);
  const load = async () => {
    const [next, status] = await Promise.all([
      call("network_report", { id: server.id }),
      call("network_check", { id: server.id }),
    ]);
    setReport(next);
    setCheck(status);
    setPort(String(next.mapping?.externalPort ?? next.port ?? server.port));
  };
  useEffect(() => {
    if (!desktop) return;
    void run(`${key}-load`, load);
  }, [server.id]);
  const toggle = () =>
    run(`${key}-toggle`, async () => {
      if (mapped) {
        setReport(await call("network_unmap", { id: server.id }));
        setMessage("");
      } else {
        const value = port.trim() ? Number(port) : undefined;
        setReport(
          await call("network_map", {
            id: server.id,
            externalPort: value,
          }),
        );
      }
    });
  const allow = () =>
    run(`${key}-allow`, async () => {
      if (!check) return;
      await call("firewall_allow", { port: check.port });
      setAllowMessage(t("network.allowSent"));
    });
  const copy = () =>
    run(`${key}-copy`, async () => {
      if (!report?.address) return;
      await navigator.clipboard.writeText(report.address);
      setMessage(t("network.copied"));
    });
  const rows = [
    {
      id: "lan",
      icon: Wifi,
      label: t("network.lan"),
      value: report?.lanIp
        ? `${report.lanIp}:${report.port}`
        : t("common.unavailable"),
    },
    {
      id: "public",
      icon: Globe,
      label: t("network.public"),
      value: report?.publicIp ?? t("common.unavailable"),
    },
    {
      id: "router",
      icon: Router,
      label: t("network.router"),
      value: report?.router ?? t("common.unavailable"),
    },
  ];
  return (
    <div className="network-stack">
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Wifi size={18} />
              {t("network.title")}
            </h2>
            <p>{t("network.desc")}</p>
          </div>
          <div className="monitor-actions">
            <span className={`net-badge ${report?.igdFound ? "ok" : ""}`}>
              {report?.igdFound
                ? t("network.igdOk")
                : t("network.igdNo")}
            </span>
            <Button
              busy={busy.has(`${key}-load`)}
              onClick={() => void run(`${key}-load`, load)}
            >
              <RefreshCw size={15} />
              {t("network.refresh")}
            </Button>
          </div>
        </div>
        {report ? (
          <>
            <div className="net-grid">
              {rows.map((row) => {
                const Icon = row.icon;
                return (
                  <article className="net-card" key={row.id}>
                    <span className="metric-icon">
                      <Icon size={16} />
                    </span>
                    <small>{row.label}</small>
                    <code>{row.value}</code>
                  </article>
                );
              })}
              <article className="net-card">
                <span className="metric-icon">
                  <Globe size={16} />
                </span>
                <small>{t("network.port")}</small>
                <code>{report.port}</code>
              </article>
            </div>
            <div className="net-share">
              <div>
                <small>{t("network.address")}</small>
                {report.address ? (
                  <code>{report.address}</code>
                ) : (
                  <p className="muted">{t("network.noAddress")}</p>
                )}
                {mapped && report.mapping?.mappedAt && (
                  <small className="muted">
                    {t("network.mappedAt")} {date(report.mapping.mappedAt)}
                  </small>
                )}
              </div>
              <Button
                disabled={!report.address || busy.has(`${key}-copy`)}
                busy={busy.has(`${key}-copy`)}
                onClick={() => void copy()}
              >
                <Copy size={15} />
                {t("network.copy")}
              </Button>
            </div>
          </>
        ) : (
          <p className="mini-empty">
            {busy.has(`${key}-load`)
              ? t("common.loading")
              : t("common.unavailable")}
          </p>
        )}
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <Globe size={18} />
              {t("network.enable")}
            </h2>
            <p>{t("network.hint")}</p>
          </div>
          <div className="task-toggle">
            <button
              type="button"
              className={`switch ${mapped ? "on" : ""}`}
              role="switch"
              aria-checked={mapped}
              aria-label={t("network.enable")}
              disabled={busy.has(`${key}-toggle`)}
              onClick={() => void toggle()}
            />
            <span>{mapped ? t("network.disable") : t("network.enable")}</span>
          </div>
        </div>
        <div className="rule-grid">
          <Field label={t("network.externalPort")} hint={t("network.externalHint")}>
            <input
              type="number"
              min={1024}
              max={65535}
              value={port}
              disabled={mapped || busy.has(`${key}-toggle`)}
              onChange={(event) => setPort(event.target.value)}
            />
          </Field>
        </div>
        {message && (
          <p className="success-text" role="status">
            {message}
          </p>
        )}
        <div className="net-fallback">
          <TriangleAlert size={18} />
          <div>
            <strong>{t("network.fallbackTitle")}</strong>
            <p className="muted">
              {t("network.fallbackDesc", { port: report?.port ?? server.port })}
            </p>
          </div>
        </div>
      </section>
      <section className="panel">
        <div className="section-title">
          <div>
            <h2>
              <ShieldCheck size={18} />
              {t("network.checkTitle")}
            </h2>
            <p>{t("network.checkDesc")}</p>
          </div>
          <Button
            busy={busy.has(`${key}-load`)}
            onClick={() => void run(`${key}-load`, load)}
          >
            <RefreshCw size={15} />
            {t("network.refresh")}
          </Button>
        </div>
        {check ? (
          <>
            <div className="net-grid">
              <article className="net-card">
                <span className="metric-icon">
                  <Wifi size={16} />
                </span>
                <small>{t("network.port")}</small>
                <code>{check.port}</code>
                <span
                  className={`net-badge ${
                    check.portFree || active(server.runtime.status) ? "ok" : ""
                  }`}
                >
                  {check.portFree
                    ? t("network.checkPortFree")
                    : active(server.runtime.status)
                      ? t("network.checkPortRunning")
                      : t("network.checkPortBusy")}
                </span>
              </article>
              <article className="net-card">
                <span className="metric-icon">
                  <ShieldCheck size={16} />
                </span>
                <small>{t("network.firewallRule")}</small>
                <code>{check.ruleName}</code>
                <span
                  className={`net-badge ${check.firewallRule ? "ok" : ""}`}
                >
                  {check.firewallRule
                    ? t("network.checkFirewallOn")
                    : t("network.checkFirewallOff")}
                </span>
              </article>
            </div>
            {!check.firewallRule && (
              <div className="net-fallback">
                <TriangleAlert size={18} />
                <div>
                  <strong>{t("network.checkFirewallOff")}</strong>
                  <p className="muted">{t("network.allowHint")}</p>
                  <Button
                    busy={busy.has(`${key}-allow`)}
                    onClick={() => void allow()}
                  >
                    <ShieldCheck size={15} />
                    {t("network.allowFirewall")}
                  </Button>
                </div>
              </div>
            )}
            {allowMessage && (
              <p className="success-text" role="status">
                {allowMessage}
              </p>
            )}
          </>
        ) : (
          <p className="mini-empty">
            {busy.has(`${key}-load`)
              ? t("common.loading")
              : t("common.unavailable")}
          </p>
        )}
      </section>
    </div>
  );
}
