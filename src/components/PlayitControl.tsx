import { useEffect, useState } from "react";
import { Copy, ExternalLink, Globe } from "lucide-react";
import { Button } from "./ui";
import { call, desktop } from "../services/api";
import { useI18n } from "../i18n";
import type { PlayitInfo } from "../types";
import type { TaskProps } from "../utils/task";

export function PlayitControl({
  id,
  run,
  busy,
  onAddress,
}: TaskProps & {
  id: string;
  onAddress: (address: string) => void;
}) {
  const { t } = useI18n();
  const [info, setInfo] = useState<PlayitInfo | null>(null);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const key = `playit-${id}`;
  useEffect(() => {
    let alive = true;
    let pending = false;
    setInfo(null);
    setError("");
    setCopied(false);
    onAddress("");
    const load = async () => {
      if (!desktop || pending) return;
      pending = true;
      try {
        const next = await call("playit_info", { id });
        if (!alive) return;
        setInfo(next);
        setError("");
        onAddress(
          next.enabled && next.phase === "online" ? (next.address ?? "") : "",
        );
      } catch {
        if (alive) {
          setError(t("playit.loadFailed"));
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
  const online = info?.enabled && info.phase === "online";
  return (
    <section className="panel">
      <div className="section-title">
        <div>
          <h2>
            <Globe size={18} /> {t("playit.title")}
          </h2>
          <p>{t("playit.desc")}</p>
        </div>
        <span className={`net-badge ${online ? "ok" : ""}`}>
          {t(`playit.phase.${info?.phase || "preparing"}`)}
        </span>
      </div>
      <p className="small muted">{t("playit.explain")}</p>
      <div className="row">
        <Button
          variant={info?.enabled ? "secondary" : "primary"}
          disabled={!desktop || !info}
          busy={busy.has(key)}
          onClick={() =>
            void run(key, async () => {
              const next = await call("playit_enable", {
                id,
                enabled: !info?.enabled,
              });
              setInfo(next);
              setCopied(false);
              onAddress("");
            })
          }
        >
          {t(info?.enabled ? "playit.disable" : "playit.enable")}
        </Button>
        {info?.enabled && info.claimUrl && !info.authorized && (
          <Button
            busy={busy.has(`${key}-claim`)}
            onClick={() =>
              void run(`${key}-claim`, () => call("playit_open_claim"))
            }
          >
            <ExternalLink size={15} /> {t("playit.authorize")}
          </Button>
        )}
      </div>
      {info?.enabled && info.claimUrl && !info.authorized && (
        <p className="info-strip">{t("playit.claimHint")}</p>
      )}
      {info?.enabled && info.phase === "ready" && (
        <p className="info-strip">{t("playit.readyHint")}</p>
      )}
      {info?.enabled && info.address && (
        <div className="net-share">
          <div>
            <small>{t("playit.address")}</small>
            <code>{info.address}</code>
            {!online && <p className="small muted">{t("playit.verifying")}</p>}
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
      {(error || (info?.enabled && info.message)) && (
        <p className="warning-text" role="status">
          {error || info?.message}
        </p>
      )}
      {info?.enabled && info.authorized && info.phase === "error" && (
        <Button
          busy={busy.has(`${key}-reset`)}
          onClick={() =>
            void run(`${key}-reset`, async () => {
              await call("playit_reset_authorization");
              setInfo(null);
              onAddress("");
            })
          }
        >
          {t("playit.reauthorize")}
        </Button>
      )}
    </section>
  );
}
