import { useEffect, useMemo, useRef, useState } from "react";
import {
  Archive,
  Bell,
  BellOff,
  Clock,
  PlayCircle,
  TriangleAlert,
} from "lucide-react";
import { useI18n, type MessageKey } from "../i18n";
import type { Notice, NoticeKind } from "../types";

const MUTE_KEY = "minehost.notices.muted";
const READ_KEY = "minehost.notices.readAt";

function stored(key: string): string {
  try {
    return localStorage.getItem(key) ?? "";
  } catch {
    return "";
  }
}

const icons: Record<NoticeKind, typeof Bell> = {
  serverStarted: PlayCircle,
  serverCrashed: TriangleAlert,
  backupComplete: Archive,
  taskFailed: Clock,
};

interface Props {
  notices: Notice[];
  onOpen: (serverId: string) => void;
  onClear: () => void;
}

export function NotificationCenter({ notices, onOpen, onClear }: Props) {
  const { t, locale } = useI18n();
  const [open, setOpen] = useState(false);
  const [muted, setMuted] = useState(() => stored(MUTE_KEY) === "1");
  const [readAt, setReadAt] = useState(() => stored(READ_KEY));
  const box = useRef<HTMLDivElement>(null);

  const markRead = () => {
    const now = new Date().toISOString();
    setReadAt(now);
    try {
      localStorage.setItem(READ_KEY, now);
    } catch {
      /* The timestamp still applies for this session. */
    }
  };

  useEffect(() => {
    if (!open) return;
    const onDown = (e: MouseEvent) => {
      if (!box.current?.contains(e.target as Node)) setOpen(false);
    };
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") setOpen(false);
    };
    document.addEventListener("mousedown", onDown);
    window.addEventListener("keydown", onKey);
    return () => {
      document.removeEventListener("mousedown", onDown);
      window.removeEventListener("keydown", onKey);
    };
  }, [open]);

  const unread = muted ? 0 : notices.filter((n) => n.at > readAt).length;

  const time = useMemo(
    () =>
      new Intl.DateTimeFormat(locale, {
        month: "2-digit",
        day: "2-digit",
        hour: "2-digit",
        minute: "2-digit",
      }),
    [locale],
  );

  const toggle = () => {
    if (!open) markRead();
    setOpen(!open);
  };

  const toggleMute = () => {
    const next = !muted;
    setMuted(next);
    try {
      localStorage.setItem(MUTE_KEY, next ? "1" : "0");
    } catch {
      /* The preference still applies for this session. */
    }
  };

  return (
    <div className="notice" ref={box}>
      <button
        className={`notice-trigger${open ? " active" : ""}`}
        onClick={toggle}
        title={t("notice.title")}
        aria-label={t("notice.title")}
        aria-expanded={open}
      >
        <Bell size={17} />
        {unread > 0 && (
          <span className="notice-count">{unread > 9 ? "9+" : unread}</span>
        )}
      </button>
      {open && (
        <div
          className="notice-panel"
          role="dialog"
          aria-label={t("notice.title")}
        >
          <div className="notice-head">
            <strong>{t("notice.title")}</strong>
            <div className="row">
              <button
                className="notice-action"
                onClick={toggleMute}
                title={muted ? t("notice.unmute") : t("notice.mute")}
                aria-label={muted ? t("notice.unmute") : t("notice.mute")}
              >
                {muted ? <BellOff size={15} /> : <Bell size={15} />}
              </button>
              <button
                className="notice-action"
                onClick={() => {
                  markRead();
                  onClear();
                }}
                disabled={!notices.length}
              >
                {t("notice.clear")}
              </button>
            </div>
          </div>
          {notices.length ? (
            <div className="notice-list">
              {notices.map((notice, index) => {
                const Icon = icons[notice.kind];
                return (
                  <button
                    key={`${notice.at}-${notice.serverId}-${notice.kind}-${index}`}
                    className={`notice-item ${notice.kind}`}
                    onClick={() => {
                      onOpen(notice.serverId);
                      setOpen(false);
                    }}
                    title={t("notice.open", { name: notice.serverName })}
                  >
                    <span className="notice-icon">
                      <Icon size={15} />
                    </span>
                    <span className="notice-body">
                      <span className="notice-title">
                        {t(`notice.kind.${notice.kind}` as MessageKey)}
                        <small>{notice.serverName}</small>
                      </span>
                      {notice.detail && (
                        <span className="notice-detail">{notice.detail}</span>
                      )}
                      <span className="notice-time">
                        {time.format(new Date(notice.at))}
                      </span>
                    </span>
                  </button>
                );
              })}
            </div>
          ) : (
            <p className="notice-empty">{t("notice.empty")}</p>
          )}
          <div className="notice-foot">
            {muted ? t("notice.muted") : t("notice.hint")}
          </div>
        </div>
      )}
    </div>
  );
}
