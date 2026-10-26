import { useEffect, useRef, useState } from "react";
import {
  Copy,
  ExternalLink,
  Globe,
  KeyRound,
  RefreshCw,
  Save,
} from "lucide-react";
import { Button, Field } from "./ui";
import { call, desktop } from "../services/api";
import { useI18n } from "../i18n";
import {
  idle,
  type PublicAccessInfo,
  type PublicAccessProtocol,
  type PublicAccessProvider,
  type SakuraNode,
  type ServerView,
  type TunnelConfig,
} from "../types";
import { phaseKey, providerKeys } from "../utils/publicAccess";
import type { TaskProps } from "../utils/task";

const providers: PublicAccessProvider[] = ["lan", "sakura", "playit", "custom"];

export function PublicAccessControl({
  server,
  run,
  busy,
  onAddress,
}: TaskProps & {
  server: ServerView;
  onAddress: (address: string) => void;
}) {
  const { t } = useI18n();
  const [info, setInfo] = useState<PublicAccessInfo | null>(null);
  const [loaded, setLoaded] = useState(false);
  const [provider, setProvider] = useState<PublicAccessProvider>("lan");
  const [protocol, setProtocol] = useState<PublicAccessProtocol>("tcp");
  const [localPort, setLocalPort] = useState("");
  const [autoPort, setAutoPort] = useState(false);
  const [node, setNode] = useState<number | null>(null);
  const [nodes, setNodes] = useState<SakuraNode[]>([]);
  const [token, setToken] = useState("");
  const [changingToken, setChangingToken] = useState(false);
  const [sakuraSaved, setSakuraSaved] = useState(false);
  const [playitSaved, setPlayitSaved] = useState(false);
  const [custom, setCustom] = useState<TunnelConfig>({
    executable: "",
    configPath: "",
    address: "",
  });
  const [error, setError] = useState("");
  const [copied, setCopied] = useState(false);
  const notify = useRef(onAddress);
  notify.current = onAddress;
  const key = `public-access-${server.id}`;
  const online = info?.phase === "online";
  const authorized = Boolean(info?.authorized);
  // While another provider is still selected, the polled info describes that
  // provider. Read the draft provider's saved credentials separately so its
  // setup controls and destructive actions are accurate before saving.
  const sakuraAuthorized =
    info?.provider === "sakura" ? authorized : sakuraSaved;
  const playitAuthorized =
    info?.provider === "playit" ? authorized : playitSaved;

  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    let pending = false;
    let first = true;
    setLoaded(false);
    setInfo(null);
    setError("");
    setCopied(false);
    setChangingToken(false);
    setToken("");
    setNodes([]);
    setNode(null);
    setSakuraSaved(false);
    setPlayitSaved(false);
    notify.current("");
    const load = async () => {
      if (pending) return;
      pending = true;
      try {
        const next = await call("public_access_info", { id: server.id });
        if (!alive) return;
        setInfo(next);
        setError("");
        notify.current(next.phase === "online" ? (next.address ?? "") : "");
        if (first) {
          first = false;
          setProvider(next.provider);
          setProtocol(next.protocol);
          // A TCP port equal to the server port means "follow the server";
          // UDP always needs an explicit port.
          setLocalPort(
            next.protocol === "udp"
              ? String(next.localPort)
              : next.localPort === server.port
                ? ""
                : String(next.localPort),
          );
          setAutoPort(false);
          setLoaded(true);
        }
      } catch {
        if (alive) {
          setError(t("public.loadFailed"));
          notify.current("");
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
  }, [server.id, server.port, t]);

  useEffect(() => {
    if (!desktop || provider !== "sakura" || info?.provider === "sakura")
      return;
    let alive = true;
    void call("sakura_info", { id: server.id })
      .then((next) => {
        if (alive) setSakuraSaved(next.authorized);
      })
      .catch(() => {
        if (alive) setSakuraSaved(false);
      });
    return () => {
      alive = false;
    };
  }, [provider, server.id, info?.provider]);

  useEffect(() => {
    if (!desktop || provider !== "playit" || info?.provider === "playit")
      return;
    let alive = true;
    void call("playit_info", { id: server.id })
      .then((next) => {
        if (alive) setPlayitSaved(next.authorized);
      })
      .catch(() => {
        if (alive) setPlayitSaved(false);
      });
    return () => {
      alive = false;
    };
  }, [provider, server.id, info?.provider]);

  useEffect(() => {
    if (!desktop || !sakuraAuthorized || provider !== "sakura") return;
    let alive = true;
    void call("sakura_nodes")
      .then((list) => {
        if (alive) setNodes(list);
      })
      .catch(() => {
        if (alive) setNodes([]);
      });
    return () => {
      alive = false;
    };
  }, [sakuraAuthorized, provider]);

  useEffect(() => {
    if (!desktop || provider !== "custom") return;
    let alive = true;
    void call("tunnel_info", { id: server.id })
      .then((next) => {
        if (!alive) return;
        setCustom((previous) =>
          previous.executable || previous.configPath || previous.address
            ? previous
            : next.config,
        );
      })
      .catch(() => {
        /* The initial info call already reports provider errors. */
      });
    return () => {
      alive = false;
    };
  }, [provider, server.id]);

  const save = () =>
    void run(key, async () => {
      if (provider === "sakura" && token.trim()) {
        await call("sakura_save_token", { token });
        setToken("");
        setChangingToken(false);
      }
      const next = await call("public_access_configure", {
        id: server.id,
        provider,
        protocol,
        localPort: localPort.trim() ? Number(localPort) : undefined,
        node: provider === "sakura" ? (node ?? undefined) : undefined,
        custom:
          provider === "custom" &&
          (custom.executable || custom.configPath || custom.address)
            ? custom
            : undefined,
      });
      setInfo(next);
      setCopied(false);
      notify.current(next.phase === "online" ? (next.address ?? "") : "");
    });

  const resetPlayit = () =>
    void run(`${key}-reset`, async () => {
      await call("playit_reset_authorization");
      setInfo(await call("public_access_info", { id: server.id }));
    });

  const changeProtocol = (next: PublicAccessProtocol) => {
    setProtocol(next);
    if (next === "udp" && !localPort.trim()) {
      setLocalPort("19132");
      setAutoPort(true);
    } else if (next === "tcp" && autoPort) {
      setLocalPort("");
      setAutoPort(false);
    }
  };

  return (
    <section className="panel">
      <div className="section-title">
        <div>
          <h2>
            <Globe size={18} /> {t("public.title")}
          </h2>
          <p>{t("public.desc")}</p>
        </div>
        <span className={`net-badge ${online ? "ok" : ""}`}>
          {t(info ? phaseKey(info.phase) : "public.phase.preparing")}
        </span>
      </div>
      <p className="small muted">{t("public.explain")}</p>

      <div className="rule-grid">
        <Field label={t("public.provider")} hint={t("public.providerHint")}>
          <select
            value={provider}
            disabled={!desktop || !loaded}
            onChange={(event) =>
              setProvider(event.target.value as PublicAccessProvider)
            }
          >
            {providers.map((id) => (
              <option key={id} value={id}>
                {t(providerKeys[id])}
              </option>
            ))}
          </select>
        </Field>
        <Field label={t("public.protocol")} hint={t("public.protocolHint")}>
          <select
            value={protocol}
            disabled={!desktop || !loaded}
            onChange={(event) =>
              changeProtocol(event.target.value as PublicAccessProtocol)
            }
          >
            <option value="tcp">{t("public.protocol.tcp")}</option>
            <option value="udp">{t("public.protocol.udp")}</option>
          </select>
        </Field>
        <Field
          label={t("public.localPort")}
          hint={t("public.localPortHint", { port: server.port })}
        >
          <input
            type="number"
            min={1}
            max={65535}
            value={localPort}
            placeholder={String(server.port)}
            disabled={!desktop || !loaded}
            onChange={(event) => {
              setLocalPort(event.target.value);
              setAutoPort(false);
            }}
          />
        </Field>
      </div>

      {provider === "sakura" && (
        <>
          {!sakuraAuthorized || changingToken ? (
            <div className="rule-grid">
              <Field
                label={t("sakura.tokenLabel")}
                hint={t("sakura.tokenHint")}
              >
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
          ) : (
            <div className="rule-grid">
              <Field label={t("sakura.nodeLabel")} hint={t("sakura.nodeHint")}>
                <select
                  value={node ?? ""}
                  onChange={(event) =>
                    setNode(
                      event.target.value ? Number(event.target.value) : null,
                    )
                  }
                >
                  <option value="">{t("sakura.nodeAuto")}</option>
                  {nodes.map((entry) => (
                    <option
                      key={entry.id}
                      value={entry.id}
                      disabled={!entry.available || entry.offline}
                    >
                      {[
                        entry.name,
                        entry.host,
                        entry.vip > 0
                          ? t("sakura.vipBadge", { level: entry.vip })
                          : "",
                        entry.publicGames ? t("sakura.publicBadge") : "",
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </option>
                  ))}
                </select>
              </Field>
            </div>
          )}
          <div className="row">
            <Button
              busy={busy.has(`${key}-token-page`)}
              onClick={() =>
                void run(`${key}-token-page`, () =>
                  call("sakura_open_token_page"),
                )
              }
            >
              <KeyRound size={15} /> {t("sakura.getToken")}
            </Button>
            <Button
              busy={busy.has(`${key}-panel`)}
              onClick={() =>
                void run(`${key}-panel`, () => call("sakura_open_panel"))
              }
            >
              <ExternalLink size={15} /> {t("sakura.manage")}
            </Button>
            {sakuraAuthorized && !changingToken && (
              <Button variant="ghost" onClick={() => setChangingToken(true)}>
                {t("sakura.changeToken")}
              </Button>
            )}
          </div>
          <p className="small muted">{t("sakura.publicRule")}</p>
        </>
      )}

      {provider === "playit" && (
        <>
          <p className="small muted">{t("public.playitHint")}</p>
          <div className="row">
            {info?.provider === "playit" && info.claimUrl && !authorized && (
              <Button
                busy={busy.has(`${key}-claim`)}
                onClick={() =>
                  void run(`${key}-claim`, () => call("playit_open_claim"))
                }
              >
                <ExternalLink size={15} /> {t("playit.authorize")}
              </Button>
            )}
            {playitAuthorized && idle(server.runtime.status) && (
              <Button
                busy={busy.has(`${key}-reset`)}
                onClick={() => void resetPlayit()}
              >
                <RefreshCw size={15} /> {t("playit.reauthorize")}
              </Button>
            )}
          </div>
        </>
      )}

      {provider === "custom" && (
        <>
          <div className="rule-grid">
            {(["executable", "configPath", "address"] as const).map((field) => (
              <Field key={field} label={t(`tunnel.${field}`)}>
                <input
                  value={custom[field]}
                  maxLength={field === "address" ? 253 : 4096}
                  onChange={(event) =>
                    setCustom((previous) => ({
                      ...previous,
                      [field]: event.target.value,
                    }))
                  }
                />
              </Field>
            ))}
          </div>
          <p className="small muted">{t("public.customHint")}</p>
        </>
      )}

      {provider === "lan" && (
        <p className="small muted">{t("public.lanHint")}</p>
      )}

      <div className="row">
        <Button
          variant="primary"
          disabled={!desktop || !loaded || busy.has(key)}
          busy={busy.has(key)}
          onClick={() => void save()}
        >
          <Save size={15} /> {t("public.save")}
        </Button>
        {info?.configured && (
          <span className="small muted">
            {t("public.savedHint", {
              provider: t(providerKeys[info.provider]),
            })}
          </span>
        )}
      </div>

      {info?.address && (
        <div className="net-share">
          <div>
            <small>{t("public.address")}</small>
            <code>{info.address}</code>
            {!online && info.phase !== "ready" && (
              <p className="small muted">{t("public.verifying")}</p>
            )}
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
            {t(copied ? "public.copied" : "public.copy")}
          </Button>
        </div>
      )}

      {info?.phase === "needs_setup" && (
        <p className="info-strip">{t("public.needsSetup")}</p>
      )}
      {info?.message && (
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
