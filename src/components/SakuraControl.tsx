import { useEffect, useState } from "react";
import { Copy, ExternalLink, Globe, KeyRound, TriangleAlert } from "lucide-react";
import { Button, Field } from "./ui";
import { call, desktop } from "../services/api";
import { useI18n } from "../i18n";
import type { SakuraInfo, SakuraNode } from "../types";
import type { TaskProps } from "../utils/task";

export function SakuraControl({
  id,
  run,
  busy,
  onAddress,
}: TaskProps & {
  id: string;
  onAddress: (address: string) => void;
}) {
  const { t } = useI18n();
  const [info, setInfo] = useState<SakuraInfo | null>(null);
  const [nodes, setNodes] = useState<SakuraNode[]>([]);
  const [node, setNode] = useState<number | null>(null);
  const [token, setToken] = useState("");
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const key = `sakura-${id}`;
  useEffect(() => {
    let alive = true;
    let pending = false;
    setInfo(null);
    setNodes([]);
    setNode(null);
    setToken("");
    setError("");
    setCopied(false);
    onAddress("");
    const load = async () => {
      if (!desktop || pending) return;
      pending = true;
      try {
        const next = await call("sakura_info", { id });
        if (!alive) return;
        setInfo(next);
        setError("");
        onAddress(
          next.enabled && next.phase === "online" ? (next.address ?? "") : "",
        );
      } catch {
        if (alive) {
          setError(t("sakura.loadFailed"));
          setInfo((previous) =>
            previous ? { ...previous, phase: "error", address: null } : null,
          );
          onAddress("");
        }
      } finally {
        pending = false;
      }
    };
    void load();
    const timer = window.setInterval(() => void load(), 3000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [id, onAddress, t]);
  const authorized = Boolean(info?.authorized);
  const enabled = Boolean(info?.enabled);
  useEffect(() => {
    if (!desktop || !authorized) return;
    let alive = true;
    void call("sakura_nodes")
      .then((list) => {
        if (!alive) return;
        setNodes(list);
        setNode(
          (previous) =>
            previous ??
            list.find((entry) => entry.available && !entry.offline)?.id ??
            null,
        );
      })
      .catch(() => {
        if (alive) setNodes([]);
      });
    return () => {
      alive = false;
    };
  }, [authorized]);
  const online = enabled && info?.phase === "online";
  const nodeLabel = (entry: SakuraNode) =>
    [
      entry.name,
      entry.host,
      entry.vip > 0 ? t("sakura.vipBadge", { level: entry.vip }) : "",
      entry.publicGames ? t("sakura.publicBadge") : "",
    ]
      .filter(Boolean)
      .join(" · ");
  const saveToken = () =>
    void run(`${key}-token`, async () => {
      await call("sakura_save_token", { token });
      setToken("");
      setInfo(await call("sakura_info", { id }));
    });
  const resetToken = () =>
    void run(`${key}-reset-token`, async () => {
      await call("sakura_reset_token");
      setInfo(null);
      setNodes([]);
      setNode(null);
      setCopied(false);
      onAddress("");
    });
  const toggle = () =>
    void run(key, async () => {
      if (!info) return;
      const next = await call("sakura_enable", {
        id,
        enabled: !info.enabled,
        node: info.enabled ? undefined : (node ?? undefined),
      });
      setInfo(next);
      setCopied(false);
      if (!next.enabled) onAddress("");
    });
  return (
    <section className="panel">
      <div className="section-title">
        <div>
          <h2>
            <Globe size={18} /> {t("sakura.title")}
          </h2>
          <p>{t("sakura.desc")}</p>
        </div>
        <span className={`net-badge ${online ? "ok" : ""}`}>
          {t(`sakura.phase.${info?.phase || "preparing"}`)}
        </span>
      </div>
      <p className="small muted">{t("sakura.explain")}</p>
      {!info ? (
        <p className="mini-empty">{t("common.loading")}</p>
      ) : !authorized ? (
        <>
          <div className="rule-grid">
            <Field label={t("sakura.tokenLabel")} hint={t("sakura.tokenHint")}>
              <input
                type="password"
                autoComplete="new-password"
                spellCheck={false}
                value={token}
                placeholder={t("sakura.tokenPlaceholder")}
                onChange={(event) => setToken(event.target.value)}
              />
            </Field>
          </div>
          <div className="row">
            <Button
              variant="primary"
              disabled={!desktop || !token.trim()}
              busy={busy.has(`${key}-token`)}
              onClick={() => void saveToken()}
            >
              <KeyRound size={15} /> {t("sakura.saveToken")}
            </Button>
            <Button
              busy={busy.has(`${key}-open-token`)}
              onClick={() =>
                void run(`${key}-open-token`, () => call("sakura_open_token_page"))
              }
            >
              <ExternalLink size={15} /> {t("sakura.getToken")}
            </Button>
          </div>
        </>
      ) : (
        <>
          {!enabled && (
            <div className="rule-grid">
              <Field label={t("sakura.nodeLabel")} hint={t("sakura.nodeHint")}>
                <select
                  value={node ?? ""}
                  onChange={(event) =>
                    setNode(event.target.value ? Number(event.target.value) : null)
                  }
                >
                  {nodes.length === 0 ? (
                    <option value="">{t("sakura.nodeUnavailable")}</option>
                  ) : (
                    nodes.map((entry) => (
                      <option
                        key={entry.id}
                        value={entry.id}
                        disabled={!entry.available || entry.offline}
                      >
                        {nodeLabel(entry)}
                      </option>
                    ))
                  )}
                </select>
              </Field>
            </div>
          )}
          {!enabled && (
            <p className="small muted">
              <TriangleAlert size={13} /> {t("sakura.publicRule")}
            </p>
          )}
          <div className="row">
            <Button
              variant={enabled ? "secondary" : "primary"}
              disabled={!desktop || !info}
              busy={busy.has(key)}
              onClick={() => void toggle()}
            >
              {t(enabled ? "sakura.disable" : "sakura.enable")}
            </Button>
            <Button
              busy={busy.has(`${key}-panel`)}
              onClick={() =>
                void run(`${key}-panel`, () => call("sakura_open_panel"))
              }
            >
              <ExternalLink size={15} /> {t("sakura.manage")}
            </Button>
            {(!enabled || info?.phase === "error") && (
              <Button
                variant="ghost"
                busy={busy.has(`${key}-reset-token`)}
                onClick={() => void resetToken()}
              >
                {t("sakura.changeToken")}
              </Button>
            )}
          </div>
        </>
      )}
      {enabled && info?.phase === "ready" && (
        <p className="info-strip">{t("sakura.readyHint")}</p>
      )}
      {enabled && info?.address && (
        <div className="net-share">
          <div>
            <small>{t("sakura.address")}</small>
            <code>{info.address}</code>
            {!online && <p className="small muted">{t("sakura.verifying")}</p>}
            {info.tunnelId != null && (
              <small className="muted">
                {t("sakura.tunnel", { id: info.tunnelId })}
              </small>
            )}
          </div>
          <Button
            disabled={!online}
            onClick={() =>
              void run(`${key}-copy`, async () => {
                await navigator.clipboard.writeText(info.address!);
                setCopied(true);
              })
            }
          >
            <Copy size={15} />
            {t(copied ? "network.copied" : "network.copy")}
          </Button>
        </div>
      )}
      {enabled && info?.message && !info.address && (
        <p className="warning-text" role="status">
          {info.message}
        </p>
      )}
      {error && (
        <p className="warning-text" role="status">
          {error}
        </p>
      )}
    </section>
  );
}
