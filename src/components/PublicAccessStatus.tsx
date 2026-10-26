import { useEffect, useState } from "react";
import { Copy, Globe, Settings2 } from "lucide-react";
import { Button } from "./ui";
import { call, desktop } from "../services/api";
import { useI18n } from "../i18n";
import type { PublicAccessInfo } from "../types";
import { phaseKey, providerKeys } from "../utils/publicAccess";

/** Compact public-access summary for the server overview. */
export function PublicAccessStatus({
  id,
  onOpen,
}: {
  id: string;
  onOpen: () => void;
}) {
  const { t } = useI18n();
  const [info, setInfo] = useState<PublicAccessInfo | null>(null);
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const online = info?.phase === "online";
  const address = online ? info?.address : null;

  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    let pending = false;
    setInfo(null);
    setError("");
    setCopied(false);
    const load = async () => {
      if (pending) return;
      pending = true;
      try {
        const next = await call("public_access_info", { id });
        if (!alive) return;
        setInfo(next);
        setError("");
      } catch {
        if (alive) setError(t("public.loadFailed"));
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
  }, [id, t]);

  if (!info && !error) return null;

  return (
    <div className="info-strip public-access-strip" role="status">
      <Globe size={18} />
      <div className="public-access-copy">
        <strong>
          {t("public.title")}
          {info ? ` · ${t(providerKeys[info.provider])}` : ""}
        </strong>
        {info && (
          <span className={`net-badge ${online ? "ok" : ""}`}>
            {t(phaseKey(info.phase))}
          </span>
        )}
        {address ? (
          <code>{address}</code>
        ) : (
          <small className="muted">{t("public.addressPending")}</small>
        )}
        {info?.message && <p className="warning-text">{info.message}</p>}
        {error && <p className="warning-text">{error}</p>}
      </div>
      <div className="row">
        <Button
          disabled={!address}
          onClick={() =>
            void navigator.clipboard
              .writeText(address!)
              .then(() => setCopied(true))
              .catch(() => setCopied(false))
          }
        >
          <Copy size={15} />
          {t(copied ? "public.copied" : "public.copy")}
        </Button>
        <Button onClick={onOpen}>
          <Settings2 size={15} /> {t("public.manage")}
        </Button>
      </div>
    </div>
  );
}
