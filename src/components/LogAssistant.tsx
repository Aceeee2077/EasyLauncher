import { useEffect, useState } from "react";
import { Sparkles } from "lucide-react";
import { Button, Field } from "./ui";
import { call } from "../services/api";
import { useI18n } from "../i18n";
import type { Settings } from "../types";
import type { TaskProps } from "../utils/task";

export function LogAssistant({ id, run, busy }: TaskProps & { id: string }) {
  const { t, locale } = useI18n();
  const [settings, setSettings] = useState<Settings | null>(null);
  const [endpoint, setEndpoint] = useState("");
  const [model, setModel] = useState("");
  const [key, setKey] = useState("");
  const [remember, setRemember] = useState(false);
  const [secretAvailable, setSecretAvailable] = useState(false);
  const [preview, setPreview] = useState<{
    excerpt: string;
    source: string | null;
  } | null>(null);
  const [answer, setAnswer] = useState("");
  const task = `log-ai-${id}`;
  useEffect(() => {
    let alive = true;
    void run(`log-ai-load-${id}`, async () => {
      const [saved, status] = await Promise.all([
        call("get_settings"),
        call("secret_status"),
      ]);
      if (!alive) return;
      setSettings(saved);
      setEndpoint(saved.assistantEndpoint ?? "");
      setModel(saved.assistantModel ?? "");
      setSecretAvailable(status.available);
      if (status.available && status.names.includes("assistant.key")) {
        const stored = await call("secret_load", { name: "assistant.key" });
        if (alive && stored) {
          setKey(stored);
          setRemember(true);
        }
      }
    });
    return () => {
      alive = false;
    };
  }, [id]);
  const rememberSettings = async () => {
    if (!settings) return;
    if (
      settings.assistantEndpoint === endpoint &&
      settings.assistantModel === model
    ) {
      return;
    }
    const updated = {
      ...settings,
      assistantEndpoint: endpoint,
      assistantModel: model,
    };
    await call("save_settings", { settings: updated });
    setSettings(updated);
  };
  return (
    <section className="panel">
      <h2>
        <Sparkles size={18} />
        {t("assistant.title")}
      </h2>
      <p className="muted">{t("assistant.desc")}</p>
      <details>
        <summary>{t("assistant.setup")}</summary>
        <p className="small muted">{t("assistant.privacy")}</p>
        <div className="form-grid">
          <Field label={t("assistant.endpoint")}>
            <input
              type="url"
              placeholder="https://your-provider.example/v1/chat/completions"
              value={endpoint}
              maxLength={2000}
              disabled={busy.has(task)}
              onChange={(e) => setEndpoint(e.target.value)}
            />
          </Field>
          <Field label={t("assistant.model")}>
            <input
              value={model}
              maxLength={200}
              disabled={busy.has(task)}
              onChange={(e) => setModel(e.target.value)}
            />
          </Field>
          <Field label={t("assistant.key")}>
            <input
              type="password"
              autoComplete="off"
              spellCheck={false}
              value={key}
              maxLength={8192}
              disabled={busy.has(task)}
              onChange={(e) => setKey(e.target.value)}
            />
          </Field>
        </div>
        {secretAvailable && (
          <label className="small inline-check">
            <input
              type="checkbox"
              checked={remember}
              disabled={busy.has(task)}
              onChange={(e) => setRemember(e.target.checked)}
            />
            {t("assistant.remember")}
          </label>
        )}
        {secretAvailable && (
          <p className="small muted">{t("assistant.rememberHint")}</p>
        )}
        <Button
          busy={busy.has(`${task}-preview`)}
          disabled={busy.has(task)}
          onClick={() =>
            void run(`${task}-preview`, async () => {
              setPreview(await call("log_assistant_preview", { id }));
              setAnswer("");
            })
          }
        >
          {t("assistant.preview")}
        </Button>
        {preview && (
          <>
            <p className="small muted">
              {preview.source ?? t("ops.noLogsTitle")}
            </p>
            <Field label={t("assistant.excerpt")} hint={t("assistant.review")}>
              <textarea
                className="raw-editor log-excerpt"
                value={preview.excerpt}
                maxLength={16000}
                disabled={busy.has(task)}
                onChange={(e) =>
                  setPreview({ ...preview, excerpt: e.target.value })
                }
              />
            </Field>
            <Button
              variant="primary"
              busy={busy.has(task)}
              disabled={
                !preview.excerpt.trim() || !endpoint.trim() || !model.trim()
              }
              onClick={() =>
                void run(task, async () => {
                  setAnswer("");
                  try {
                    if (secretAvailable) {
                      if (remember) {
                        await call("secret_save", {
                          name: "assistant.key",
                          value: key,
                        });
                      } else {
                        await call("secret_delete", {
                          name: "assistant.key",
                        });
                      }
                    }
                    await rememberSettings();
                    setAnswer(
                      await call("analyze_log", {
                        id,
                        endpoint,
                        model,
                        key,
                        excerpt: preview.excerpt,
                        language: locale,
                      }),
                    );
                  } finally {
                    if (!remember) setKey("");
                  }
                })
              }
            >
              <Sparkles size={16} />
              {t("assistant.send")}
            </Button>
          </>
        )}
        {answer && (
          <div className="assistant-answer" role="status">
            <p className="small muted">{t("assistant.caution")}</p>
            <pre>{answer}</pre>
          </div>
        )}
      </details>
    </section>
  );
}
