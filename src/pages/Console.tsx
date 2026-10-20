import { useEffect, useRef, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import { ArrowDown, Send, Trash2, Terminal } from "lucide-react";
import { Button } from "../components/ui";
import { call } from "../services/api";
import type { ConsoleBatch, ConsoleLine, ServerView } from "../types";
import type { TaskProps } from "../utils/task";
import { useI18n } from "../i18n";
export function Console({
  server,
  run,
  busy,
}: TaskProps & { server: ServerView }) {
  const { t, locale } = useI18n();
  const [lines, setLines] = useState<ConsoleLine[]>([]);
  const [auto, setAuto] = useState(true);
  const [command, setCommand] = useState("");
  const bottom = useRef<HTMLDivElement>(null);
  const history = useRef<string[]>([]);
  const cursor = useRef(0);
  useEffect(() => {
    let alive = true;
    let ready = false;
    let pending: ConsoleLine[] = [];
    let floor = 0;
    setLines([]);
    const commit = (incoming: ConsoleLine[]) => {
      const unique = new Map<number, ConsoleLine>();
      for (const line of incoming)
        if (line.sequence > floor) unique.set(line.sequence, line);
      if (!unique.size) return;
      const fresh = Array.from(unique.values()).sort(
        (a, b) => a.sequence - b.sequence,
      );
      floor = fresh[fresh.length - 1].sequence;
      setLines((old) => {
        const next = old.concat(fresh);
        return next.length > 1500 ? next.slice(next.length - 1500) : next;
      });
    };
    const subscription = listen<ConsoleBatch>(
      "server-console-batch",
      (e) => {
        if (!alive || e.payload.id !== server.id) return;
        if (!ready) pending.push(...e.payload.lines);
        else commit(e.payload.lines);
      },
    );
    void run(`history-${server.id}-${crypto.randomUUID()}`, async () => {
      await subscription;
      const logs = await call("console_history", { id: server.id });
      if (!alive) return;
      commit(logs.concat(pending));
      pending = [];
      ready = true;
    });
    return () => {
      alive = false;
      void subscription.then((unlisten) => unlisten());
    };
  }, [server.id]);
  useEffect(() => {
    if (auto) bottom.current?.scrollIntoView({ block: "end" });
  }, [lines, auto]);
  const canSend = ["running", "starting"].includes(server.runtime.status);
  return (
    <section className="console-card">
      <div className="console-toolbar">
        <span>
          <Terminal size={16} />
          {t("console.live")}{" "}
          <span className="muted small">
            {t("console.lines", { shown: lines.length })}
          </span>
        </span>
        <div className="row">
          <label className="small inline-check">
            <input
              type="checkbox"
              checked={auto}
              onChange={(e) => setAuto(e.target.checked)}
            />
            {t("console.follow")}
          </label>
          <Button
            variant="ghost"
            onClick={() =>
              void run("clear", async () => {
                await call("clear_console", { id: server.id });
                setLines([]);
              })
            }
            title={t("console.clear")}
          >
            <Trash2 size={16} />
          </Button>
        </div>
      </div>
      <div
        className="console-output"
        role="log"
        aria-label={t("console.aria")}
        aria-live="off"
        onScroll={(e) => {
          const el = e.currentTarget;
          if (el.scrollHeight - el.scrollTop - el.clientHeight > 90)
            setAuto(false);
        }}
      >
        {!lines.length && (
          <div className="console-welcome">
            <Terminal size={30} />
            <p>{t("console.emptyTitle")}</p>
            <small>{t("console.emptyDesc")}</small>
          </div>
        )}
        {lines.map((l) => (
          <div
            className={`console-line ${l.stream} ${/\b(ERROR|Exception|FATAL)\b/.test(l.text) ? "log-error" : ""}`}
            key={l.sequence}
          >
            <time>{new Date(l.timestamp).toLocaleTimeString(locale)}</time>
            <span>
              {l.stream === "input" ? "> " : ""}
              {l.text}
            </span>
          </div>
        ))}
        <div ref={bottom} />
      </div>
      {!auto && (
        <button className="jump" onClick={() => setAuto(true)}>
          <ArrowDown size={14} />
          {t("console.followLatest")}
        </button>
      )}
      <form
        className="console-input"
        onSubmit={(e) => {
          e.preventDefault();
          const sent = command.trim();
          if (!sent) return;
          void run("send", async () => {
            await call("send_console_command", {
              id: server.id,
              command: sent,
            });
            history.current.push(sent);
            cursor.current = history.current.length;
            setCommand("");
          });
        }}
      >
        <span className="prompt">›</span>
        <input
          aria-label={t("console.commandAria")}
          disabled={!canSend}
          placeholder={
            canSend
              ? t("console.commandPlaceholder")
              : t("console.startPlaceholder")
          }
          value={command}
          onChange={(e) => setCommand(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "ArrowUp" || e.key === "ArrowDown") {
              e.preventDefault();
              cursor.current = Math.max(
                0,
                Math.min(
                  history.current.length,
                  cursor.current + (e.key === "ArrowUp" ? -1 : 1),
                ),
              );
              setCommand(history.current[cursor.current] || "");
            }
          }}
        />
        <Button
          type="submit"
          variant="primary"
          disabled={!canSend || !command.trim()}
          busy={busy.has("send")}
        >
          <Send size={15} />
          {t("console.send")}
        </Button>
      </form>
    </section>
  );
}
