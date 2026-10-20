import { useCallback, useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  LayoutDashboard,
  Server as ServerIcon,
  Users,
  Archive,
  Terminal,
  Settings,
  Plus,
  ArrowUpRight,
  ArrowLeft,
  Play,
  Square,
  RotateCw,
  FolderOpen,
  Trash2,
  RefreshCw,
  Layers,
  Cpu,
  Network,
  Clock,
  ChevronRight,
  Mountain,
  Activity,
  Download,
  ShieldCheck,
  Stethoscope,
  Package,
  Globe,
  FolderInput,
  LayoutGrid,
  TriangleAlert,
  Search,
} from "lucide-react";
import {
  Button,
  Empty,
  ErrorNotice,
  Modal,
  StatusBadge,
  bytes,
} from "./components/ui";
import { CreateWizard } from "./pages/CreateWizard";
import { ImportServer } from "./pages/ImportServer";
import { Console } from "./pages/Console";
import { ServerSettings } from "./pages/ServerSettings";
import { Players } from "./pages/Players";
import { Backups } from "./pages/Backups";
import { Schedule } from "./pages/Schedule";
import { Monitor } from "./pages/Monitor";
import { NetworkPanel } from "./pages/Network";
import { Ops } from "./pages/Ops";
import { Overview } from "./pages/Overview";
import { Content } from "./pages/Content";
import { AppSettings } from "./pages/AppSettings";
import { Splash } from "./components/Splash";
import { CommandPalette } from "./components/CommandPalette";
import { NotificationCenter } from "./components/NotificationCenter";
import { useTask } from "./hooks/useTask";
import { useI18n, type MessageKey } from "./i18n";
import { brand, call, desktop } from "./services/api";
import { applyAppearance } from "./services/appearance";
import {
  active,
  idle,
  type AppError,
  type ConsoleLine,
  type HealthCheck,
  type Notice,
  type Progress,
  type Server,
  type ServerView,
  type Snapshot,
  type SystemInfo,
} from "./types";
type Page =
  | "dashboard"
  | "overview"
  | "servers"
  | "players"
  | "backups"
  | "console"
  | "settings"
  | "detail";
const nav = [
  ["dashboard", "nav.dashboard", LayoutDashboard],
  ["overview", "nav.overview", LayoutGrid],
  ["servers", "nav.servers", ServerIcon],
  ["players", "nav.players", Users],
  ["backups", "nav.backups", Archive],
  ["console", "nav.console", Terminal],
] as const;
const shortcut =
  typeof navigator !== "undefined" && /mac/i.test(navigator.platform)
    ? "⌘K"
    : "Ctrl K";
export default function App() {
  const { t, setLanguage } = useI18n();
  const { error, setError, busy, run } = useTask();
  const [servers, setServers] = useState<ServerView[]>([]);
  const [info, setInfo] = useState<SystemInfo | null>(null);
  const [page, setPage] = useState<Page>("dashboard");
  const [selected, setSelected] = useState("");
  const [wizard, setWizard] = useState(false);
  const [importOpen, setImportOpen] = useState(false);
  const [tab, setTab] = useState("console");
  const [progress, setProgress] = useState<Record<string, Progress>>({});
  const [confirmDelete, setConfirmDelete] = useState<ServerView | null>(null);
  const [guard, setGuard] = useState<{
    server: ServerView;
    checks: HealthCheck[];
  } | null>(null);
  const [log, setLog] = useState<string | null>(null);
  const [paletteOpen, setPaletteOpen] = useState(false);
  const [notices, setNotices] = useState<Notice[]>([]);
  const [loading, setLoading] = useState(desktop);
  const [closing, setClosing] = useState(false);
  const load = useCallback(async () => {
    const list = await call("list_servers");
    setServers(list);
    setSelected((s) => (list.some((v) => v.id === s) ? s : list[0]?.id || ""));
  }, []);
  useEffect(() => {
    if (!desktop) return;
    let alive = true;
    const subscriptions = [
      listen<Snapshot>("server-status", (e) => {
        if (!alive) return;
        setServers((s) =>
          s.map((v) =>
            v.id === e.payload.id ? { ...v, runtime: e.payload } : v,
          ),
        );
      }),
      listen<Progress>("download-progress", (e) => {
        if (alive) setProgress((p) => ({ ...p, [e.payload.id]: e.payload }));
      }),
      listen<Notice>("notice", (e) => {
        if (alive) {
          setNotices((list) => [e.payload, ...list].slice(0, 50));
        }
      }),
      listen<void>("app-closing", () => setClosing(true)),
      listen<AppError>("app-error", (e) => {
        if (alive) {
          setError(e.payload);
          setClosing(false);
        }
      }),
    ];
    void run(`initialize-${crypto.randomUUID()}`, async () => {
      await Promise.all(subscriptions);
      await load();
      const [system, settings] = await Promise.all([
        call("system_info"),
        call("get_settings"),
      ]);
      if (alive) {
        setInfo(system);
        setLanguage(settings.language);
        await applyAppearance(settings.appearance);
      }
    }).finally(() => {
      if (alive) setLoading(false);
    });
    return () => {
      alive = false;
      subscriptions.forEach((s) => void s.then((f) => f()));
    };
  }, []);
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if ((e.ctrlKey || e.metaKey) && e.key.toLowerCase() === "k") {
        e.preventDefault();
        setPaletteOpen((open) => !open);
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);
  const server = servers.find((s) => s.id === selected);
  const running = servers.filter((s) => s.runtime.status === "running").length;
  const manage = (s: ServerView, section = "console") => {
    setSelected(s.id);
    setTab(section);
    setPage("detail");
  };
  const control = (
    s: ServerView,
    action: "start_server" | "stop_server" | "restart_server",
  ) =>
    void run(`control-${s.id}`, async () => {
      try {
        await call(action, { id: s.id });
      } catch (e) {
        if (
          action === "start_server" &&
          (e as AppError).code === "server.preflightBlocked"
        ) {
          setGuard({
            server: s,
            checks: await call("start_preflight", { id: s.id }),
          });
          return;
        }
        throw e;
      }
      await load();
    });
  const forceStart = () => {
    if (!guard) return;
    const target = guard.server;
    setGuard(null);
    void run(`control-${target.id}`, async () => {
      await call("start_server", { id: target.id, force: true });
      await load();
    });
  };
  const install = (s: Server | ServerView) =>
    void run(`install-${s.id}`, async () => {
      await call("install_server", { id: s.id });
      await load();
    });
  const created = (s: Server) => {
    setWizard(false);
    void run("created", async () => {
      await load();
      setSelected(s.id);
      setPage("detail");
      setTab("console");
      install(s);
    });
  };
  const imported = (s: Server) => {
    setImportOpen(false);
    void run("imported", async () => {
      await load();
      setSelected(s.id);
      setPage("detail");
      setTab("console");
    });
  };
  const shared = { run, busy };
  const online = server?.runtime.status === "running";
  function serverControls(s: ServerView) {
    const state = s.runtime.status;
    return (
      <div className="row controls">
        <Button
          variant="ghost"
          aria-label={t("detail.openFolder")}
          title={t("detail.openFolder")}
          onClick={() =>
            void run("folder", () =>
              call("open_folder", { id: s.id, kind: "server" }),
            )
          }
        >
          <FolderOpen size={18} />
        </Button>
        {s.installed ? (
          <>
            {active(state) ? (
              <Button
                disabled={state === "stopping"}
                busy={busy.has(`control-${s.id}`)}
                onClick={() => control(s, "stop_server")}
              >
                <Square size={15} />
                {t("common.stop")}
              </Button>
            ) : (
              <Button
                variant="primary"
                disabled={!idle(state)}
                busy={busy.has(`control-${s.id}`)}
                onClick={() => control(s, "start_server")}
              >
                <Play size={15} />
                {t("detail.startServer")}
              </Button>
            )}
            <Button
              disabled={!["running", "crashed", "stopped"].includes(state)}
              busy={busy.has(`control-${s.id}`)}
              onClick={() => control(s, "restart_server")}
              aria-label={t("detail.restart")}
              title={t("detail.restart")}
            >
              <RotateCw size={17} />
            </Button>
          </>
        ) : state === "installing" ? (
          <Button
            onClick={() =>
              void run("cancel", () => call("cancel_install", { id: s.id }))
            }
          >
            {t("detail.installCancel")}
          </Button>
        ) : (
          <Button
            variant="primary"
            busy={busy.has(`install-${s.id}`)}
            onClick={() => install(s)}
          >
            <Download size={16} />
            {t("detail.installRetry")}
          </Button>
        )}
      </div>
    );
  }
  function downloadProgress(s: ServerView) {
    const p = progress[s.id];
    const stage = (value?: string) => {
      if (!value) return t("progress.resolving");
      if (value.startsWith("Downloading "))
        return t("install.downloading", {
          label: value.slice("Downloading ".length),
        });
      if (value === "Download verified") return t("install.verified");
      return value;
    };
    return s.runtime.status === "installing" ? (
      <div className="download">
        <div>
          <span>{stage(p?.stage)}</span>
          <strong>
            {p?.totalBytes
              ? `${Math.min(100, Math.floor((p.downloadedBytes / p.totalBytes) * 100))}%`
              : ""}
          </strong>
        </div>
        <progress
          max={p?.totalBytes || undefined}
          value={p?.totalBytes ? p.downloadedBytes : undefined}
        />
        <small>
          {p
            ? `${bytes(p.downloadedBytes)}${p.totalBytes ? ` / ${bytes(p.totalBytes)}` : ""} · ${t("progress.attempt", { n: p.attempt })}`
            : t("progress.preparing")}
        </small>
      </div>
    ) : null;
  }
  return (
    <>
      <Splash />
      <div className="app-shell">
      <aside className="sidebar">
        <button className="brand" onClick={() => setPage("dashboard")}>
          <div className="brand-mark">
            <Mountain size={23} strokeWidth={2.5} />
          </div>
          <span>
            {brand.name}
            <small>{t("app.serverManager")}</small>
          </span>
        </button>
        <div className="workspace-label">{t("app.workspace")}</div>
        <nav>
          {nav.map(([id, title, Icon]) => (
            <button
              key={id}
              className={
                page === id || (id === "servers" && page === "detail")
                  ? "active"
                  : ""
              }
              onClick={() => setPage(id)}
            >
              <Icon size={19} />
              <span>{t(title)}</span>
              {id === "servers" && <small>{servers.length}</small>}
            </button>
          ))}
        </nav>
        <div className="sidebar-world">
          <div className="world-lines">
            <Layers size={26} />
          </div>
          <strong>{t("sidebar.tagline")}</strong>
          <p>{t("sidebar.description")}</p>
          <Button onClick={() => setWizard(true)} disabled={!desktop}>
            <Plus size={15} />
            {t("common.newServer")}
          </Button>
        </div>
        <div className="sidebar-bottom">
          <button
            className={page === "settings" ? "active" : ""}
            onClick={() => setPage("settings")}
          >
            <Settings size={18} />
            {t("common.settings")}
          </button>
          <div className="version">
            <span>
              <i />
              {t("app.localWorkspace")}
            </span>
            <small>v{brand.version}</small>
          </div>
        </div>
      </aside>
      <div className="main-shell">
        <header className="topbar">
          <div className="breadcrumb">
            {t("topbar.workspace")}
            <ChevronRight size={14} />
            <strong>
              {page === "detail"
                ? server?.name || t("topbar.server")
                : page === "settings"
                  ? t("common.settings")
                  : t(nav.find((n) => n[0] === page)![1])}
            </strong>
          </div>
          <div className="row">
            <span className="local-label">
              <ShieldCheck size={14} />
              {t("topbar.hostedLocally")}
            </span>
            <NotificationCenter
              notices={notices}
              onClear={() => setNotices([])}
              onOpen={(id) => {
                const target = servers.find((s) => s.id === id);
                if (target) manage(target, "console");
              }}
            />
            <button
              className="palette-trigger"
              disabled={!desktop}
              onClick={() => setPaletteOpen(true)}
              title={t("palette.trigger")}
            >
              <Search size={15} />
              <span>{t("palette.trigger")}</span>
              <kbd>{shortcut}</kbd>
            </button>
            <Button
              variant="ghost"
              aria-label={t("topbar.refresh")}
              disabled={!desktop}
              busy={busy.has("refresh")}
              onClick={() => void run("refresh", load)}
            >
              <RefreshCw size={17} />
            </Button>
          </div>
        </header>
        <main>
          {!desktop && (
            <div className="browser-banner">
              <Terminal size={18} />
              <span>
                {t("browser.required")} <code>npm run tauri dev</code>{" "}
                {t("browser.after")}
              </span>
            </div>
          )}
          {error && (
            <ErrorNotice error={error} onClose={() => setError(null)} />
          )}
          {closing && (
            <div className="info-strip">{t("app.closing")}</div>
          )}
          {(page === "dashboard" || page === "servers") && (
            <>
              <div className="page-heading">
                <div>
                  <div className="eyebrow">{t("page.eyebrow")}</div>
                  <h1>
                    {page === "dashboard"
                      ? t("page.dashboardTitle")
                      : t("page.serversTitle")}
                  </h1>
                  <p>{t("page.subtitle")}</p>
                </div>
                <div className="page-actions">
                  <Button
                    disabled={!desktop}
                    onClick={() => setImportOpen(true)}
                  >
                    <FolderInput size={17} />
                    {t("import.title")}
                  </Button>
                  <Button
                    variant="primary"
                    disabled={!desktop}
                    onClick={() => setWizard(true)}
                  >
                    <Plus size={17} />
                    {t("common.createServer")}
                  </Button>
                </div>
              </div>
              <div className="stats-grid">
                <div className="stat">
                  <span className="stat-icon">
                    <ServerIcon size={21} />
                  </span>
                  <div>
                    <small>{t("stats.totalServers")}</small>
                    <strong>
                      {servers.length.toString().padStart(2, "0")}
                    </strong>
                  </div>
                  <span className="stat-note">{t("stats.inWorkspace")}</span>
                </div>
                <div className="stat">
                  <span className="stat-icon green">
                    <Activity size={21} />
                  </span>
                  <div>
                    <small>{t("stats.runningNow")}</small>
                    <strong>{running.toString().padStart(2, "0")}</strong>
                  </div>
                  <span className="stat-note">
                    {running ? t("stats.readyForAdventure") : t("stats.allQuiet")}
                  </span>
                </div>
                <div className="stat">
                  <span className="stat-icon">
                    <Cpu size={21} />
                  </span>
                  <div>
                    <small>{t("stats.systemMemory")}</small>
                    <strong>
                      {info ? `${(info.availableMb / 1024).toFixed(1)}` : "—"}
                      <em>{t("stats.gbFree")}</em>
                    </strong>
                  </div>
                  <span className="stat-note">
                    {info
                      ? t("stats.gbTotal", {
                          size: (info.totalMb / 1024).toFixed(1),
                        })
                      : t("stats.desktopRequired")}
                  </span>
                </div>
              </div>
              <div className="section-heading">
                <h2>
                  {t("section.yourServers")} <span>{servers.length}</span>
                </h2>
                <small>{t("section.everything")}</small>
              </div>
              {loading ? (
                <div className="panel muted">{t("section.loadingServers")}</div>
              ) : servers.length ? (
                <div className="server-grid">
                  {servers.map((s, i) => (
                    <article key={s.id} className="server-card">
                      <div className={`server-art art-${i % 3}`}>
                        <div className="terrain terrain-back" />
                        <div className="terrain terrain-front" />
                        <div className="art-sun" />
                        <div className="software-tag">
                          <Layers size={13} />
                          {s.serverType}
                        </div>
                        <StatusBadge status={s.runtime.status} />
                      </div>
                      <div className="server-card-body">
                        <div className="server-title">
                          <div>
                            <h3>{s.name}</h3>
                            <p>
                              {t("server.subtitle", {
                                version: s.minecraftVersion,
                              })}
                            </p>
                          </div>
                          <button
                            className="icon-arrow"
                            onClick={() => manage(s)}
                            aria-label={t("server.manageAria", { name: s.name })}
                          >
                            <ArrowUpRight size={20} />
                          </button>
                        </div>
                        <div className="server-meta">
                          <span>
                            <Cpu size={14} />
                            {t("server.ram", {
                              size: (s.memoryMaxMb / 1024).toFixed(1),
                            })}
                          </span>
                          <span>
                            <Network size={14} />
                            {s.port}
                          </span>
                        </div>
                        {downloadProgress(s)}
                        {s.runtime.message &&
                          ["crashed", "installationFailed"].includes(
                            s.runtime.status,
                          ) && (
                            <p className="card-error">{s.runtime.message}</p>
                          )}
                        <div className="card-footer">
                          <Button onClick={() => manage(s)}>
                            {t("common.manage")}
                            <ChevronRight size={14} />
                          </Button>
                          {s.installed ? (
                            <Button
                              variant={
                                active(s.runtime.status)
                                  ? "secondary"
                                  : "primary"
                              }
                              disabled={
                                s.runtime.status === "stopping" ||
                                (!idle(s.runtime.status) &&
                                  !active(s.runtime.status))
                              }
                              busy={busy.has(`control-${s.id}`)}
                              onClick={() =>
                                control(
                                  s,
                                  active(s.runtime.status)
                                    ? "stop_server"
                                    : "start_server",
                                )
                              }
                            >
                              {active(s.runtime.status) ? (
                                <>
                                  <Square size={13} />
                                  {t("common.stop")}
                                </>
                              ) : (
                                <>
                                  <Play size={13} />
                                  {t("common.start")}
                                </>
                              )}
                            </Button>
                          ) : (
                            <Button
                              disabled={s.runtime.status === "installing"}
                              onClick={() => install(s)}
                            >
                              {t("common.install")}
                            </Button>
                          )}
                        </div>
                      </div>
                    </article>
                  ))}
                  <button
                    className="create-card"
                    onClick={() => setWizard(true)}
                  >
                    <span>
                      <Plus size={23} />
                    </span>
                    <strong>{t("server.newWorldAwaits")}</strong>
                    <small>{t("server.createAnother")}</small>
                  </button>
                </div>
              ) : (
                <section className="welcome-card">
                  <div className="welcome-art">
                    <Mountain size={96} strokeWidth={1} />
                    <div className="orbit one" />
                    <div className="orbit two" />
                  </div>
                  <div className="eyebrow">
                    {t("welcome.eyebrow", { name: brand.name.toUpperCase() })}
                  </div>
                  <h2>{t("welcome.tagline")}</h2>
                  <p>
                    {t("welcome.description")}
                    <br />
                    {t("welcome.description2")}
                  </p>
                  <div className="welcome-actions">
                    <Button
                      variant="primary"
                      disabled={!desktop}
                      onClick={() => setWizard(true)}
                    >
                      <Plus size={17} />
                      {t("welcome.createFirst")}
                      <ArrowUpRight size={16} />
                    </Button>
                    <Button
                      disabled={!desktop}
                      onClick={() => setImportOpen(true)}
                    >
                      <FolderInput size={16} />
                      {t("import.title")}
                    </Button>
                  </div>
                  <div className="welcome-notes">
                    <span>
                      <ShieldCheck size={14} />
                      {t("welcome.localFiles")}
                    </span>
                    <span>
                      <Layers size={14} />
                      {t("welcome.software")}
                    </span>
                  </div>
                </section>
              )}
              <div className="workspace-footer">
                <span>
                  <ShieldCheck size={15} />
                  {t("footer.locallyManaged")}
                </span>
                <small>{t("footer.disclaimer")}</small>
              </div>
            </>
          )}
          {page === "overview" && (
            <>
              <div className="page-heading">
                <div>
                  <div className="eyebrow">{t("overview.eyebrow")}</div>
                  <h1>{t("overview.title")}</h1>
                  <p>{t("overview.subtitle")}</p>
                </div>
                <div className="page-actions">
                  <Button
                    variant="primary"
                    disabled={!desktop}
                    onClick={() => setWizard(true)}
                  >
                    <Plus size={17} />
                    {t("common.createServer")}
                  </Button>
                </div>
              </div>
              <Overview
                {...shared}
                servers={servers}
                reload={load}
                onManage={manage}
              />
            </>
          )}
          {page === "settings" && (
            <>
              <div className="page-heading">
                <div>
                  <div className="eyebrow">{t("settings.eyebrow")}</div>
                  <h1>{t("settings.title")}</h1>
                  <p>{t("settings.subtitle")}</p>
                </div>
              </div>
              <AppSettings
                {...shared}
                info={info}
                onAppearance={(a) => {
                  void run("appearance", () => applyAppearance(a));
                }}
              />
            </>
          )}
          {["detail", "console", "players", "backups"].includes(page) && (
            <>
              {!server ? (
                <Empty
                  title={t("empty.chooseWorld")}
                  description={t("empty.chooseWorldDesc")}
                  action={
                    <Button
                      variant="primary"
                      disabled={!desktop}
                      onClick={() => setWizard(true)}
                    >
                      {t("common.createServer")}
                    </Button>
                  }
                />
              ) : (
                <>
                  <div className="detail-top">
                    <button
                      className="back-link"
                      onClick={() => setPage("servers")}
                    >
                      <ArrowLeft size={15} />
                      {t("detail.allServers")}
                    </button>
                    <select
                      aria-label={t("detail.selectServer")}
                      value={selected}
                      onChange={(e) => setSelected(e.target.value)}
                    >
                      {servers.map((s) => (
                        <option key={s.id} value={s.id}>
                          {s.name}
                        </option>
                      ))}
                    </select>
                  </div>
                  <div className="page-heading detail-heading">
                    <div>
                      <div className="row">
                        <h1>{server.name}</h1>
                        <StatusBadge status={server.runtime.status} />
                      </div>
                      <p>
                        {t("detail.meta", {
                          type: server.serverType,
                          version: server.minecraftVersion,
                          java: server.requiredJava || "—",
                        })}
                      </p>
                    </div>
                    {serverControls(server)}
                  </div>
                  <div className="connection-strip">
                    <span>
                      <Network size={16} />
                      <small>{t("detail.localAddress")}</small>
                      <code>localhost:{server.port}</code>
                    </span>
                    <span>
                      <small>{t("detail.lanAddress")}</small>
                      <code>
                        {info?.lanAddress
                          ? `${info.lanAddress}:${server.port}`
                          : t("common.unavailable")}
                      </code>
                    </span>
                    <span>
                      <Cpu size={16} />
                      <small>{t("detail.ramLimit")}</small>
                      <code>{server.memoryMaxMb} MB</code>
                    </span>
                    {active(server.runtime.status) && (
                      <span>
                        <Clock size={16} />
                        <small>{t("detail.started")}</small>
                        <code>
                          {server.runtime.startedAt
                            ? new Date(
                                server.runtime.startedAt,
                              ).toLocaleTimeString()
                            : "—"}
                        </code>
                      </span>
                    )}
                  </div>
                  <p className="address-note">{t("detail.addressNote")}</p>
                  {server.runtime.message && (
                    <div
                      className={`info-strip ${["crashed", "installationFailed"].includes(server.runtime.status) ? "crash-strip" : ""}`}
                    >
                      <Activity size={18} />
                      <span>{server.runtime.message}</span>
                      {server.runtime.status === "crashed" && (
                        <>
                          <Button
                            onClick={() =>
                              void run("latestLog", async () =>
                                setLog(
                                  await call("latest_log", { id: server.id }),
                                ),
                              )
                            }
                          >
                            {t("detail.viewLog")}
                          </Button>
                          <Button onClick={() => setTab("ops")}>
                            {t("detail.diagnose")}
                          </Button>
                        </>
                      )}
                    </div>
                  )}
                  {downloadProgress(server)}
                  {page === "detail" && (
                    <div className="tabs">
                      {[
                        ["console", "common.console", Terminal],
                        ["settings", "common.settings", Settings],
                        ["players", "common.players", Users],
                        ["backups", "common.backups", Archive],
                        ["schedule", "common.schedule", Clock],
                        ["monitor", "common.monitor", Activity],
                        ["network", "common.network", Globe],
                        ["content", "common.content", Package],
                        ["ops", "common.ops", Stethoscope],
                      ].map(([v, label, Icon]) => {
                        const TabIcon = Icon as typeof Terminal;
                        return (
                          <button
                            className={tab === v ? "active" : ""}
                            key={v as string}
                            onClick={() => setTab(v as string)}
                          >
                            <TabIcon size={16} />
                            {t(label as MessageKey)}
                          </button>
                        );
                      })}
                    </div>
                  )}
                  {(page === "console" ||
                    (page === "detail" && tab === "console")) && (
                    <Console key={server.id} {...shared} server={server} />
                  )}
                  {(page === "players" ||
                    (page === "detail" && tab === "players")) && (
                    <Players key={server.id} {...shared} server={server} />
                  )}
                  {(page === "backups" ||
                    (page === "detail" && tab === "backups")) && (
                    <Backups key={server.id} {...shared} server={server} />
                  )}
                  {page === "detail" && tab === "content" && (
                    <Content key={server.id} {...shared} server={server} />
                  )}
                  {page === "detail" && tab === "schedule" && (
                    <Schedule key={server.id} {...shared} server={server} />
                  )}
                  {page === "detail" && tab === "monitor" && (
                    <Monitor key={server.id} {...shared} server={server} />
                  )}
                  {page === "detail" && tab === "network" && (
                    <NetworkPanel key={server.id} {...shared} server={server} />
                  )}
                  {page === "detail" && tab === "ops" && (
                    <Ops key={server.id} {...shared} server={server} />
                  )}
                  {page === "detail" && tab === "settings" && (
                    <>
                      <ServerSettings
                        key={server.id}
                        {...shared}
                        server={server}
                        onSaved={load}
                      />
                      <div className="danger-zone">
                        <div>
                          <strong>{t("detail.deleteServer")}</strong>
                          <p>{t("detail.deleteServerDesc")}</p>
                        </div>
                        <Button
                          variant="danger"
                          disabled={!idle(server.runtime.status)}
                          onClick={() => setConfirmDelete(server)}
                        >
                          <Trash2 size={15} />
                          {t("detail.deleteServerButton")}
                        </Button>
                      </div>
                    </>
                  )}
                  {online && (
                    <div className="workspace-footer">
                      <span>
                        <i className="online-dot" />
                        {t("detail.startedUp")}
                      </span>
                    </div>
                  )}
                </>
              )}
            </>
          )}
        </main>
      </div>
      <CommandPalette
        open={paletteOpen}
        onClose={() => setPaletteOpen(false)}
        servers={servers}
        onPage={setPage}
        onManage={manage}
        onControl={control}
        onNew={() => setWizard(true)}
        onImport={() => setImportOpen(true)}
        onRefresh={() => void run("refresh", load)}
      />
      {wizard && (
        <CreateWizard
          {...shared}
          info={info}
          onClose={() => setWizard(false)}
          onCreated={created}
        />
      )}
      {importOpen && (
        <ImportServer
          {...shared}
          onClose={() => setImportOpen(false)}
          onImported={imported}
        />
      )}
      {confirmDelete && (
        <Modal
          title={t("delete.title", { name: confirmDelete.name })}
          onClose={() => setConfirmDelete(null)}
        >
          <p>{t("delete.body")}</p>
          <div className="modal-actions">
            <Button onClick={() => setConfirmDelete(null)}>
              {t("delete.keep")}
            </Button>
            <Button
              variant="danger"
              busy={busy.has("deleteServer")}
              onClick={() =>
                void run("deleteServer", async () => {
                  await call("delete_server", {
                    id: confirmDelete.id,
                    confirmed: true,
                  });
                  setConfirmDelete(null);
                  await load();
                  setPage("servers");
                })
              }
            >
              {t("delete.confirm")}
            </Button>
          </div>
        </Modal>
      )}
      {guard && (
        <Modal
          title={t("preflight.title", { name: guard.server.name })}
          onClose={() => setGuard(null)}
        >
          <p>{t("preflight.body")}</p>
          <div className="health-list">
            {guard.checks.map((check) => (
              <div className={`health-row ${check.level}`} key={check.id}>
                <span className={`health-dot ${check.level}`} />
                <div>
                  <strong>
                    {t(`health.${check.id}.title` as MessageKey)}
                  </strong>
                  <p>
                    {t(
                      `health.${check.id}.${check.level}` as MessageKey,
                      check.values,
                    )}
                  </p>
                </div>
              </div>
            ))}
          </div>
          <p>{t("preflight.note")}</p>
          <div className="modal-actions">
            <Button onClick={() => setGuard(null)}>{t("common.cancel")}</Button>
            <Button
              variant="danger"
              busy={busy.has(`control-${guard.server.id}`)}
              onClick={forceStart}
            >
              <TriangleAlert size={15} />
              {t("preflight.force")}
            </Button>
          </div>
        </Modal>
      )}
      {log !== null && (
        <Modal
          title={t("log.title")}
          onClose={() => setLog(null)}
          wide
        >
          <pre className="log-view">{log}</pre>
        </Modal>
      )}
      </div>
    </>
  );
}
