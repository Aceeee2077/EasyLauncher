import { useEffect, useState, type CSSProperties } from "react";
import { brand } from "../services/api";
import { BrandMark } from "./BrandMark";
import { useI18n } from "../i18n";

const HOLD = 3000;
const EXIT = 560;
const reduced = () =>
  window.matchMedia("(prefers-reduced-motion: reduce)").matches;

export function Splash() {
  const { t } = useI18n();
  const [calm] = useState(reduced);
  const [phase, setPhase] = useState<"hold" | "leave" | "done">("hold");
  useEffect(() => {
    if (phase !== "hold") return;
    const timer = window.setTimeout(
      () => setPhase("leave"),
      calm ? 900 : HOLD,
    );
    return () => window.clearTimeout(timer);
  }, [phase, calm]);
  useEffect(() => {
    if (phase !== "leave") return;
    const timer = window.setTimeout(
      () => setPhase("done"),
      calm ? 0 : EXIT,
    );
    return () => window.clearTimeout(timer);
  }, [phase, calm]);
  useEffect(() => {
    if (phase === "done") return;
    const skip = () => setPhase("leave");
    window.addEventListener("keydown", skip);
    return () => window.removeEventListener("keydown", skip);
  }, [phase]);
  if (phase === "done") return null;
  return (
    <div
      className={`splash ${phase === "leave" ? "leaving" : ""}`}
      style={{ "--splash-hold": `${HOLD}ms` } as CSSProperties}
      onClick={() => setPhase("leave")}
    >
      <div className="splash-scene" aria-hidden="true">
        <span className="splash-ink ink-a" />
        <span className="splash-ink ink-b" />
        <span className="splash-ink ink-c" />
        <span className="splash-ridge ridge-a" />
        <span className="splash-ridge ridge-b" />
      </div>
      <div className="splash-content" role="status" aria-label={brand.name}>
        <div className="splash-mark">
          <BrandMark size={32} />
        </div>
        <h1 className="splash-word">{brand.name}</h1>
        <div className="splash-rule" />
        <p className="splash-note">{t("splash.loading")}</p>
        <div className="splash-progress">
          <span />
        </div>
      </div>
      <span className="splash-skip">{t("splash.skip")}</span>
    </div>
  );
}
