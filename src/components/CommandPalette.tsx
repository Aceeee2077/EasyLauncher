import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import {
  Activity,
  Archive,
  Clock,
  Compass,
  FolderInput,
  Globe,
  LayoutDashboard,
  LayoutGrid,
  Package,
  Play,
  Plus,
  RefreshCw,
  Search,
  Server as ServerIcon,
  Settings,
  Square,
  Stethoscope,
  Terminal,
  Users,
  type LucideIcon,
} from "lucide-react";
import { useI18n, type MessageKey } from "../i18n";
import { active } from "../types";
import type { ServerView } from "../types";

export type PalettePage =
  | "dashboard"
  | "overview"
  | "servers"
  | "discover"
  | "players"
  | "backups"
  | "console"
  | "settings";
interface Props {
  open: boolean;
  onClose: () => void;
  servers: ServerView[];
  onPage: (page: PalettePage) => void;
  onManage: (server: ServerView, section: string) => void;
  onControl: (
    server: ServerView,
    action: "start_server" | "stop_server",
  ) => void;
  onNew: () => void;
  onImport: () => void;
  onRefresh: () => void;
}
interface Item {
  id: string;
  group: string;
  label: string;
  hint?: string;
  keywords: string;
  Icon: LucideIcon;
  /** Section shortcuts stay hidden until the user types, to keep the default list short. */
  secondary?: boolean;
  run: () => void;
}
const pages = [
  ["dashboard", "nav.dashboard", LayoutDashboard],
  ["overview", "nav.overview", LayoutGrid],
  ["servers", "nav.servers", ServerIcon],
  ["discover", "nav.discover", Compass],
  ["players", "nav.players", Users],
  ["backups", "nav.backups", Archive],
  ["console", "nav.console", Terminal],
  ["settings", "common.settings", Settings],
] as const satisfies readonly (readonly [PalettePage, MessageKey, LucideIcon])[];
const sections = [
  ["console", "common.console", Terminal],
  ["settings", "common.settings", Settings],
  ["players", "common.players", Users],
  ["backups", "common.backups", Archive],
  ["schedule", "common.schedule", Clock],
  ["monitor", "common.monitor", Activity],
  ["network", "common.network", Globe],
  ["content", "common.content", Package],
  ["ops", "common.ops", Stethoscope],
] as const satisfies readonly (readonly [string, MessageKey, LucideIcon])[];
export function CommandPalette({
  open,
  onClose,
  servers,
  onPage,
  onManage,
  onControl,
  onNew,
  onImport,
  onRefresh,
}: Props) {
  const { t } = useI18n();
  const [query, setQuery] = useState("");
  const [index, setIndex] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const items = useMemo(() => {
    const build: Item[] = [];
    pages.forEach(([page, label, Icon]) =>
      build.push({
        id: `page-${page}`,
        group: t("palette.group.pages"),
        label: t(label),
        keywords: `${page} ${label}`,
        Icon,
        run: () => onPage(page),
      }),
    );
    servers.forEach((server) => {
      build.push({
        id: `server-${server.id}`,
        group: t("palette.group.servers"),
        label: t("palette.open", { name: server.name }),
        hint: `${server.serverType} · ${server.minecraftVersion} · ${server.port}`,
        keywords: `${server.name} ${server.id} ${server.serverType} ${server.minecraftVersion} ${server.port}`,
        Icon: ServerIcon,
        run: () => onManage(server, "console"),
      });
      sections.forEach(([section, label, Icon]) =>
        build.push({
          id: `server-${server.id}-${section}`,
          group: t("palette.group.servers"),
          label: `${server.name} · ${t(label)}`,
          keywords: `${server.name} ${section} ${label}`,
          Icon,
          secondary: true,
          run: () => onManage(server, section),
        }),
      );
      const running = active(server.runtime.status);
      build.push({
        id: `server-${server.id}-power`,
        group: t("palette.group.actions"),
        label: running
          ? t("palette.stop", { name: server.name })
          : t("palette.start", { name: server.name }),
        hint: server.runtime.status,
        keywords: `${server.name} start stop ${server.runtime.status}`,
        Icon: running ? Square : Play,
        run: () =>
          onControl(server, running ? "stop_server" : "start_server"),
      });
    });
    build.push(
      {
        id: "action-new",
        group: t("palette.group.actions"),
        label: t("palette.newServer"),
        keywords: "new create server",
        Icon: Plus,
        run: onNew,
      },
      {
        id: "action-import",
        group: t("palette.group.actions"),
        label: t("palette.importServer"),
        keywords: "import migrate folder",
        Icon: FolderInput,
        run: onImport,
      },
      {
        id: "action-refresh",
        group: t("palette.group.actions"),
        label: t("palette.refresh"),
        keywords: "refresh reload",
        Icon: RefreshCw,
        run: onRefresh,
      },
    );
    return build;
  }, [t, servers, onPage, onManage, onControl, onNew, onImport, onRefresh]);
  const visible = useMemo(() => {
    const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);
    if (!terms.length) return items.filter((item) => !item.secondary);
    return items.filter((item) => {
      const haystack =
        `${item.label} ${item.hint || ""} ${item.keywords}`.toLowerCase();
      return terms.every((term) => haystack.includes(term));
    });
  }, [items, query]);
  useEffect(() => {
    if (!open) return;
    setQuery("");
    setIndex(0);
    const timer = window.setTimeout(() => input.current?.focus(), 20);
    return () => window.clearTimeout(timer);
  }, [open]);
  useEffect(() => setIndex(0), [query]);
  useEffect(() => {
    if (!open) return;
    list.current
      ?.querySelector<HTMLElement>(`[data-index="${index}"]`)
      ?.scrollIntoView({ block: "nearest" });
  }, [index, open]);
  if (!open) return null;
  const choose = (i: number) => {
    const item = visible[i];
    if (!item) return;
    onClose();
    item.run();
  };
  return (
    <div
      className="palette-backdrop"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <div
        className="palette"
        role="dialog"
        aria-modal="true"
        aria-label={t("palette.title")}
      >
        <div className="palette-search">
          <Search size={17} />
          <input
            ref={input}
            value={query}
            placeholder={t("palette.placeholder")}
            aria-label={t("palette.placeholder")}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Escape") {
                e.preventDefault();
                onClose();
              } else if (e.key === "ArrowDown") {
                e.preventDefault();
                setIndex((i) =>
                  Math.min(i + 1, Math.max(0, visible.length - 1)),
                );
              } else if (e.key === "ArrowUp") {
                e.preventDefault();
                setIndex((i) => Math.max(i - 1, 0));
              } else if (e.key === "Enter") {
                e.preventDefault();
                choose(index);
              }
            }}
          />
          <kbd>Esc</kbd>
        </div>
        <div className="palette-list" ref={list}>
          {visible.length ? (
            visible.map((item, i) => (
              <Fragment key={item.id}>
                {item.group !== visible[i - 1]?.group && (
                  <div className="palette-group">{item.group}</div>
                )}
                <button
                  type="button"
                  data-index={i}
                  className={i === index ? "palette-item active" : "palette-item"}
                  onMouseEnter={() => setIndex(i)}
                  onClick={() => choose(i)}
                >
                  <item.Icon size={16} />
                  <span>{item.label}</span>
                  {item.hint && <small>{item.hint}</small>}
                </button>
              </Fragment>
            ))
          ) : (
            <p className="palette-empty">{t("palette.empty")}</p>
          )}
        </div>
        <div className="palette-foot">{t("palette.hint")}</div>
      </div>
    </div>
  );
}
