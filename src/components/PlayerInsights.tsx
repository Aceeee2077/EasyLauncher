import { useEffect, useState } from "react";
import { BarChart3, RefreshCw } from "lucide-react";
import { Button, date } from "./ui";
import { call } from "../services/api";
import { useI18n } from "../i18n";
import type { PlayerStats } from "../types";
import type { TaskProps } from "../utils/task";

export function PlayerInsights({ id, run, busy }: TaskProps & { id: string }) {
  const { t } = useI18n();
  const [stats, setStats] = useState<PlayerStats | null>(null);
  const [query, setQuery] = useState("");
  const key = `player-insights-${id}`;
  useEffect(() => {
    let alive = true;
    const load = () =>
      void run(`${key}-${crypto.randomUUID()}`, async () => {
        const next = await call("player_stats", { id });
        if (alive) setStats(next);
      });
    load();
    const timer = window.setInterval(load, 15000);
    return () => {
      alive = false;
      window.clearInterval(timer);
    };
  }, [id]);
  const players = Object.values(stats?.players ?? {}).sort(
    (a, b) => b.seconds - a.seconds || b.visits - a.visits,
  );
  const duration = (seconds: number) =>
    t("insights.duration", {
      hours: Math.floor(seconds / 3600),
      minutes: Math.floor((seconds % 3600) / 60),
    });
  const now = new Date();
  const days = Array.from({ length: 14 }, (_, n) => {
    const d = new Date(now);
    d.setUTCDate(d.getUTCDate() - (13 - n));
    const key = d.toISOString().slice(0, 10);
    return stats?.days[key] ?? { date: key, joins: 0, seconds: 0 };
  });
  const peak = Math.max(1, ...days.map((d) => d.joins));
  return (
    <section className="panel insights-panel">
      <div className="section-title">
        <div>
          <h2>
            <BarChart3 size={18} />
            {t("insights.title")}
          </h2>
          <p>{t("insights.desc")}</p>
        </div>
        <Button
          busy={busy.has(key)}
          onClick={() =>
            void run(key, async () =>
              setStats(await call("player_stats", { id })),
            )
          }
        >
          <RefreshCw size={15} />
          {t("common.refresh")}
        </Button>
      </div>
      {stats ? (
        <>
          <div className="insight-totals">
            <div>
              <small>{t("insights.unique")}</small>
              <strong>{players.length}</strong>
            </div>
            <div>
              <small>{t("insights.visits")}</small>
              <strong>{players.reduce((n, p) => n + p.visits, 0)}</strong>
            </div>
            <div>
              <small>{t("insights.playtime")}</small>
              <strong>
                {duration(players.reduce((n, p) => n + p.seconds, 0))}
              </strong>
            </div>
          </div>
          <h3>{t("insights.chart")}</h3>
          <div
            className="visits-chart"
            role="img"
            aria-label={days.map((d) => `${d.date}: ${d.joins}`).join("; ")}
          >
            {days.map((d) => (
              <div
                className="visits-column"
                key={d.date}
                title={`${d.date} UTC · ${d.joins} · ${duration(d.seconds)}`}
              >
                <span>{d.joins}</span>
                <div
                  className="visits-bar"
                  style={{ height: `${Math.max(2, (d.joins / peak) * 110)}px` }}
                />
                <small>{d.date.slice(5)}</small>
              </div>
            ))}
          </div>
          <p className="small muted">
            {t("insights.since", { date: date(stats.trackingSince) })}
          </p>
          <label className="field">
            <span>{t("insights.search")}</span>
            <input
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              maxLength={16}
            />
          </label>
          <div className="table-scroll">
            <table className="insights-table">
              <thead>
                <tr>
                  <th>{t("players.name")}</th>
                  <th>{t("insights.visits")}</th>
                  <th>{t("insights.playtime")}</th>
                  <th>{t("insights.lastSeen")}</th>
                </tr>
              </thead>
              <tbody>
                {players
                  .filter((p) =>
                    p.name.toLowerCase().includes(query.toLowerCase()),
                  )
                  .slice(0, 100)
                  .map((p) => (
                    <tr key={p.name}>
                      <td>{p.name}</td>
                      <td>{p.visits}</td>
                      <td>{duration(p.seconds)}</td>
                      <td>{date(p.lastSeen)}</td>
                    </tr>
                  ))}
              </tbody>
            </table>
          </div>
          {!players.length && (
            <p className="mini-empty">{t("insights.empty")}</p>
          )}
          {players.length > 100 && (
            <p className="small muted">{t("insights.searchHint")}</p>
          )}
          <details>
            <summary>{t("insights.sessions")}</summary>
            <div className="table-scroll">
              <table className="insights-table">
                <thead>
                  <tr>
                    <th>{t("players.name")}</th>
                    <th>{t("insights.joined")}</th>
                    <th>{t("insights.left")}</th>
                    <th>{t("insights.playtime")}</th>
                  </tr>
                </thead>
                <tbody>
                  {stats.sessions
                    .slice(-30)
                    .reverse()
                    .map((s, i) => (
                      <tr key={`${s.name}-${s.joinedAt}-${i}`}>
                        <td>{s.name}</td>
                        <td>{date(s.joinedAt)}</td>
                        <td>{date(s.leftAt)}</td>
                        <td>{duration(s.seconds)}</td>
                      </tr>
                    ))}
                </tbody>
              </table>
            </div>
          </details>
        </>
      ) : (
        <p>{t("common.loading")}</p>
      )}
    </section>
  );
}
