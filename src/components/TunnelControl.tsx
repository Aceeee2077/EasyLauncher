import { useEffect, useState } from "react";
import { Globe, Play, Square } from "lucide-react";
import { Button, Field } from "./ui";
import { call } from "../services/api";
import { useI18n } from "../i18n";
import type { TunnelConfig, TunnelInfo } from "../types";
import type { TaskProps } from "../utils/task";

export function TunnelControl({
  id,
  run,
  busy,
  onAddress,
}: TaskProps & { id: string; onAddress: (address: string) => void }) {
  const { t } = useI18n();
  const [info, setInfo] = useState<TunnelInfo | null>(null);
  const [config, setConfig] = useState<TunnelConfig>({
    executable: "",
    configPath: "",
    address: "",
  });
  const key = `tunnel-${id}`;
  useEffect(() => {
    let alive = true;
    let first = true;
    let pending = false;
    const load = () => {
      if (pending) return;
      pending = true;
      void run(`${key}-poll-${crypto.randomUUID()}`, async () => {
        try {
          const next = await call("tunnel_info", { id });
          if (!alive) return;
          setInfo(next);
          if (first) {
            setConfig(next.config);
            first = false;
          }
          onAddress(next.running ? next.config.address : "");
        } finally {
          pending = false;
        }
      });
    };
    load();
    const timer = window.setInterval(load, 5000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [id, onAddress]);
  const working = busy.has(key);
  return (
    <section className="panel">
      <div className="section-title">
        <div>
          <h2>
            <Globe size={18} />
            {t("tunnel.title")}
          </h2>
          <p>{t("tunnel.desc")}</p>
        </div>
        <span className="tag">
          {info?.running
            ? t("tunnel.running", { pid: info.pid ?? "—" })
            : t("tunnel.stopped")}
        </span>
      </div>
      <p className="small muted">{t("tunnel.setup")}</p>
      <fieldset className="form-grid" disabled={info?.running || working}>
        {(["executable", "configPath", "address"] as const).map((field) => (
          <Field key={field} label={t(`tunnel.${field}`)}>
            <input
              value={config[field]}
              maxLength={field === "address" ? 253 : 4096}
              onChange={(e) =>
                setConfig((s) => ({ ...s, [field]: e.target.value }))
              }
            />
          </Field>
        ))}
      </fieldset>
      <Button
        variant={info?.running ? "secondary" : "primary"}
        busy={working}
        disabled={
          !info ||
          (!info.running &&
            (!config.executable || !config.configPath || !config.address))
        }
        onClick={() =>
          void run(key, async () => {
            const next = info?.running
              ? await call("tunnel_stop", { id })
              : await call("tunnel_start", { id, config });
            setInfo(next);
            onAddress(next.running ? next.config.address : "");
          })
        }
      >
        {info?.running ? <Square size={15} /> : <Play size={15} />}
        {t(info?.running ? "tunnel.stop" : "tunnel.start")}
      </Button>
      {info?.running && (
        <p className="info-strip break-anywhere">
          {info.config.address} · {t("tunnel.unverified")}
        </p>
      )}
      {!info?.running && info?.exitCode != null && (
        <p className="warning-text">
          {t("tunnel.exited", { code: info.exitCode })}
        </p>
      )}
    </section>
  );
}
