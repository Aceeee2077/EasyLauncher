import { useState } from "react";
import { UserRound, ShieldCheck } from "lucide-react";
import { Button, Field } from "../components/ui";
import { PlayerInsights } from "../components/PlayerInsights";
import { call } from "../services/api";
import type { ServerView } from "../types";
import type { TaskProps } from "../utils/task";
import { useI18n, type MessageKey } from "../i18n";
const actions = [
  "op",
  "deop",
  "kick",
  "ban",
  "pardon",
  "whitelistAdd",
  "whitelistRemove",
] as const;
export function Players({
  server,
  run,
  busy,
}: TaskProps & { server: ServerView }) {
  const { t } = useI18n();
  const [player, setPlayer] = useState("");
  const [action, setAction] = useState("whitelistAdd");
  const [sent, setSent] = useState("");
  const ready = server.runtime.status === "running";
  const actionLabel = (value: string) =>
    t(`players.action.${value}` as MessageKey);
  return (
    <div className="insights-stack">
      <div className="two-columns">
        <section className="panel">
          <div className="section-title">
            <div>
              <h2>{t("players.connected")}</h2>
              <p>{t("players.connectedDesc")}</p>
            </div>
            <span className="count">{server.runtime.players.length}</span>
          </div>
          {server.runtime.players.length ? (
            server.runtime.players.map((p) => (
              <button
                key={p}
                className="player-row"
                onClick={() => setPlayer(p)}
              >
                <div className="avatar">
                  <UserRound size={20} />
                </div>
                <strong>{p}</strong>
                <span className="status running">
                  <i />
                  {t("players.connectedState")}
                </span>
              </button>
            ))
          ) : (
            <div className="mini-empty">
              <UserRound size={32} />
              <p>{t("players.none")}</p>
              <small>{t("players.noneHint")}</small>
            </div>
          )}
        </section>
        <section className="panel">
          <h2>
            <ShieldCheck size={18} />
            {t("players.controls")}
          </h2>
          <p className="muted">{t("players.controlsDesc")}</p>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              void run("player", async () => {
                await call("player_action", { id: server.id, player, action });
                setSent(
                  t("players.sent", { player, action: actionLabel(action) }),
                );
              });
            }}
          >
            <Field label={t("players.name")}>
              <input
                pattern="[A-Za-z0-9_]{1,16}"
                maxLength={16}
                required
                value={player}
                placeholder={t("players.namePlaceholder")}
                onChange={(e) => setPlayer(e.target.value)}
              />
            </Field>
            <Field label={t("players.action")}>
              <select
                value={action}
                onChange={(e) => setAction(e.target.value)}
              >
                {actions.map((value) => (
                  <option key={value} value={value}>
                    {actionLabel(value)}
                  </option>
                ))}
              </select>
            </Field>
            {["op", "ban", "kick"].includes(action) && (
              <p className="warning-text small">{t("players.warning")}</p>
            )}
            <Button
              variant="primary"
              type="submit"
              busy={busy.has("player")}
              disabled={!ready || !player}
            >
              {t("players.send")}
            </Button>
          </form>
          {!ready && <p className="small muted">{t("players.notReady")}</p>}
          {sent && (
            <p className="success-text small" role="status">
              {sent}
            </p>
          )}
        </section>
      </div>
      <PlayerInsights id={server.id} run={run} busy={busy} />
    </div>
  );
}
