import { useEffect, useMemo, useRef, useState } from "react";
import { Bookmark, Download, FileSearch, RefreshCw } from "lucide-react";
import { Button, Field, bytes } from "./ui";
import { call } from "../services/api";
import { useI18n } from "../i18n";
import type { ActivityKind, ActivityLibrary, ActivityReport } from "../types";
import type { TaskProps } from "../utils/task";

const kinds: ActivityKind[] = [
  "join",
  "leave",
  "death",
  "advancement",
  "chat",
  "warning",
  "error",
  "lifecycle",
];
const PAGE_SIZE = 100;

export function ActivityWorkbench({
  id,
  run,
  busy,
}: TaskProps & { id: string }) {
  const { t } = useI18n();
  const [library, setLibrary] = useState<ActivityLibrary | null>(null);
  const [source, setSource] = useState("");
  const [report, setReport] = useState<ActivityReport | null>(null);
  const [kind, setKind] = useState("");
  const [player, setPlayer] = useState("");
  const [query, setQuery] = useState("");
  const [hour, setHour] = useState<number | null>(null);
  const [marks, setMarks] = useState<Set<number>>(new Set());
  const [markedOnly, setMarkedOnly] = useState(false);
  const [page, setPage] = useState(0);
  const generation = useRef(0);
  const loadKey = `activity-library-${id}`;
  const analyzeKey = `activity-analyze-${id}`;
  const working = busy.has(loadKey) || busy.has(analyzeKey);
  useEffect(() => {
    let alive = true;
    void run(loadKey, async () => {
      const result = await call("activity_library", { id });
      if (alive) {
        setLibrary(result);
        setSource(result.files[0]?.name ?? "");
      }
    });
    return () => {
      alive = false;
      generation.current++;
    };
  }, [id]);
  const clearAnalysis = () => {
    generation.current++;
    setReport(null);
    setMarks(new Set());
    setMarkedOnly(false);
    setKind("");
    setPlayer("");
    setQuery("");
    setHour(null);
    setPage(0);
  };
  const refresh = () => {
    clearAnalysis();
    void run(loadKey, async () => {
      const result = await call("activity_library", { id });
      setLibrary(result);
      setSource((current) =>
        result.files.some((f) => f.name === current)
          ? current
          : (result.files[0]?.name ?? ""),
      );
    });
  };
  const analyze = () => {
    clearAnalysis();
    const version = generation.current;
    void run(analyzeKey, async () => {
      const result = await call("activity_analyze", { id, name: source });
      if (version === generation.current) setReport(result);
    });
  };
  const events = useMemo(() => {
    const needle = query.trim().toLowerCase();
    return (report?.events ?? []).filter(
      (e) =>
        (!kind || e.kind === kind) &&
        (!player || e.player === player) &&
        (hour === null || e.hour === hour) &&
        (!markedOnly || marks.has(e.line)) &&
        (!needle || e.message.toLowerCase().includes(needle)),
    );
  }, [report, kind, player, query, hour, markedOnly, marks]);
  const counts = useMemo(() => {
    const result = Object.fromEntries(kinds.map((k) => [k, 0])) as Record<
      ActivityKind,
      number
    >;
    for (const event of report?.events ?? []) result[event.kind]++;
    return result;
  }, [report]);
  const changeFilter = (fn: () => void) => {
    fn();
    setPage(0);
  };
  const pageCount = Math.max(1, Math.ceil(events.length / PAGE_SIZE));
  const currentPage = Math.min(page, pageCount - 1);
  const exportReport = () => {
    if (!report) return;
    const blob = new Blob(
      [
        JSON.stringify(
          {
            schema: "easylauncher-activity-v1",
            generatedAt: new Date().toISOString(),
            source: report.source,
            scannedLines: report.scannedLines,
            truncated: report.truncated,
            filters: { kind, player, query, hour, markedOnly },
            // Export exactly the filtered selection; no hidden log lines or keys.
            events: events.map((event) => ({
              ...event,
              bookmarked: marks.has(event.line),
            })),
          },
          null,
          2,
        ),
      ],
      { type: "application/json" },
    );
    const url = URL.createObjectURL(blob);
    const link = document.createElement("a");
    link.href = url;
    link.download = `activity-${report.source.replace(/[^a-zA-Z0-9._-]/g, "_")}.json`;
    link.click();
    setTimeout(() => URL.revokeObjectURL(url), 1000);
  };
  return (
    <section className="panel activity-workbench">
      <div className="section-title">
        <div>
          <h2>
            <FileSearch size={18} />
            {t("activity.title")}
          </h2>
          <p>{t("activity.desc")}</p>
        </div>
        <Button busy={busy.has(loadKey)} disabled={working} onClick={refresh}>
          <RefreshCw size={15} />
          {t("ops.refresh")}
        </Button>
      </div>
      <div className="activity-toolbar">
        <Field label={t("activity.source")}>
          <select
            value={source}
            disabled={working}
            onChange={(e) => {
              clearAnalysis();
              setSource(e.target.value);
            }}
          >
            {!library?.files.length && (
              <option value="">{t("activity.noLogs")}</option>
            )}
            {library?.files.map((file) => (
              <option key={file.name} value={file.name}>
                {file.name} · {bytes(file.bytes)}
              </option>
            ))}
          </select>
        </Field>
        <Button
          variant="primary"
          busy={busy.has(analyzeKey)}
          disabled={!source || working}
          onClick={analyze}
        >
          {t("activity.analyze")}
        </Button>
      </div>
      {(library?.omitted ?? 0) > 0 && (
        <p>{t("activity.omitted", { count: library!.omitted })}</p>
      )}
      {report ? (
        <>
          <p className="small muted">
            {t("activity.scope", {
              lines: report.scannedLines,
              events: report.events.length,
              players: report.players.length,
            })}
          </p>
          {report.truncated && (
            <p role="status" className="warning-note">
              {t("activity.truncated")}
            </p>
          )}
          <div className="activity-counts">
            {kinds.map((k) => (
              <button
                key={k}
                aria-pressed={kind === k}
                onClick={() => changeFilter(() => setKind(kind === k ? "" : k))}
              >
                <span>{t(`activity.kind.${k}`)}</span>
                <strong>{counts[k]}</strong>
              </button>
            ))}
          </div>
          <h3>{t("activity.hours")}</h3>
          <div
            className="activity-hours"
            role="group"
            aria-label={t("activity.hours")}
          >
            {report.hours.map((count, h) => (
              <button
                key={h}
                aria-label={t("activity.hour", { hour: h, count })}
                aria-pressed={hour === h}
                title={t("activity.hour", { hour: h, count })}
                onClick={() =>
                  changeFilter(() => setHour(hour === h ? null : h))
                }
              >
                <span
                  className="activity-bar"
                  style={{
                    height: `${Math.max(3, (count / Math.max(1, ...report.hours)) * 64)}px`,
                  }}
                />
                <small>{h}</small>
              </button>
            ))}
          </div>
          <div className="activity-toolbar">
            <Field label={t("activity.player")}>
              <select
                value={player}
                onChange={(e) => changeFilter(() => setPlayer(e.target.value))}
              >
                <option value="">{t("activity.allPlayers")}</option>
                {report.players.map((p) => (
                  <option key={p}>{p}</option>
                ))}
              </select>
            </Field>
            <Field label={t("activity.search")}>
              <input
                value={query}
                onChange={(e) => changeFilter(() => setQuery(e.target.value))}
              />
            </Field>
            <label>
              <input
                type="checkbox"
                checked={markedOnly}
                onChange={(e) =>
                  changeFilter(() => setMarkedOnly(e.target.checked))
                }
              />
              {t("activity.bookmarked")}
            </label>
            <Button disabled={!events.length} onClick={exportReport}>
              <Download size={15} />
              {t("activity.export")}
            </Button>
            <Button
              onClick={() => {
                setKind("");
                setPlayer("");
                setQuery("");
                setHour(null);
                setMarkedOnly(false);
                setPage(0);
              }}
            >
              {t("activity.reset")}
            </Button>
          </div>
          <p className="small muted">{t("activity.markHint")}</p>
          <div className="activity-table-wrap">
            <table className="activity-table">
              <thead>
                <tr>
                  <th>{t("activity.line")}</th>
                  <th>{t("activity.time")}</th>
                  <th>{t("activity.type")}</th>
                  <th>{t("activity.player")}</th>
                  <th>{t("activity.evidence")}</th>
                  <th>{t("activity.mark")}</th>
                </tr>
              </thead>
              <tbody>
                {events
                  .slice(currentPage * PAGE_SIZE, (currentPage + 1) * PAGE_SIZE)
                  .map((e) => (
                    <tr key={e.line}>
                      <td>{e.line}</td>
                      <td>{e.time ?? "—"}</td>
                      <td>{t(`activity.kind.${e.kind}`)}</td>
                      <td>{e.player ?? "—"}</td>
                      <td>{e.message}</td>
                      <td>
                        <Button
                          aria-label={t("activity.markLine", { line: e.line })}
                          aria-pressed={marks.has(e.line)}
                          onClick={() =>
                            setMarks((previous) => {
                              const next = new Set(previous);
                              if (next.has(e.line)) next.delete(e.line);
                              else next.add(e.line);
                              return next;
                            })
                          }
                        >
                          <Bookmark
                            size={15}
                            fill={marks.has(e.line) ? "currentColor" : "none"}
                          />
                        </Button>
                      </td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
          {!events.length && (
            <p className="mini-empty">{t("activity.empty")}</p>
          )}
          <div className="activity-pagination">
            <Button
              disabled={currentPage === 0}
              onClick={() => setPage(currentPage - 1)}
            >
              {t("activity.previous")}
            </Button>
            <span aria-live="polite">
              {t("activity.page", {
                page: currentPage + 1,
                total: pageCount,
                count: events.length,
              })}
            </span>
            <Button
              disabled={currentPage + 1 >= pageCount}
              onClick={() => setPage(currentPage + 1)}
            >
              {t("activity.next")}
            </Button>
          </div>
        </>
      ) : (
        <p className="mini-empty">{t("activity.startHint")}</p>
      )}
    </section>
  );
}
