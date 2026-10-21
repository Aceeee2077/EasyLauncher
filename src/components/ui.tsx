import {
  useEffect,
  useRef,
  type ReactNode,
  type ButtonHTMLAttributes,
} from "react";
import {
  X,
  LoaderCircle,
  ArrowUpRight,
  Check,
  Server as ServerIcon,
} from "lucide-react";
import type { Status, AppError } from "../types";
import { useI18n, type MessageKey } from "../i18n";
import { errorMessageKey } from "../i18n/messages";
export function Button({
  children,
  variant = "secondary",
  busy = false,
  ...props
}: ButtonHTMLAttributes<HTMLButtonElement> & {
  variant?: "primary" | "secondary" | "danger" | "ghost" | "player";
  busy?: boolean;
}) {
  return (
    <button
      {...props}
      disabled={props.disabled || busy}
      className={`btn ${variant} ${props.className || ""}`}
    >
      {busy ? <LoaderCircle size={16} className="spin" /> : null}
      {children}
    </button>
  );
}
export function StatusBadge({ status }: { status: Status }) {
  const { t } = useI18n();
  return (
    <span className={`status ${status}`}>
      <i />
      {t(`status.${status}` as MessageKey)}
    </span>
  );
}
export function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint?: string;
  children: ReactNode;
}) {
  return (
    <label className="field">
      <span>{label}</span>
      {children}
      {hint && <small>{hint}</small>}
    </label>
  );
}
export function Modal({
  title,
  children,
  onClose,
  wide = false,
}: {
  title: string;
  children: ReactNode;
  onClose: () => void;
  wide?: boolean;
}) {
  const ref = useRef<HTMLElement>(null);
  const { t } = useI18n();
  useEffect(() => {
    if (!wide) return;
    // Scale text, controls and spacing together when the desktop window grows.
    const resize = () => {
      const scale = Math.max(
        1,
        Math.min(window.innerWidth / 1100, window.innerHeight / 800, 1.8),
      );
      ref.current?.style.setProperty("--modal-scale", String(scale));
    };
    resize();
    window.addEventListener("resize", resize);
    return () => window.removeEventListener("resize", resize);
  }, [wide]);
  useEffect(() => {
    const previous = document.activeElement as HTMLElement | null;
    const overflow = document.body.style.overflow;
    document.body.style.overflow = "hidden";
    ref.current?.focus();
    return () => {
      document.body.style.overflow = overflow;
      previous?.focus();
    };
  }, []);
  return (
    <div
      className="modal-scrim"
      onMouseDown={(e) => {
        if (e.target === e.currentTarget) onClose();
      }}
    >
      <section
        ref={ref}
        tabIndex={-1}
        className={`modal ${wide ? "wide" : ""}`}
        role="dialog"
        aria-modal="true"
        aria-label={title}
        onKeyDown={(e) => {
          if (e.key === "Escape") onClose();
          if (e.key === "Tab") {
            const items = Array.from(
              ref.current?.querySelectorAll<HTMLElement>(
                'button:not(:disabled), input:not(:disabled), select:not(:disabled), textarea:not(:disabled), [tabindex="0"]',
              ) || [],
            ).filter((el) => el.getClientRects().length);
            const first = items[0],
              last = items.at(-1);
            if (
              e.shiftKey &&
              (document.activeElement === first ||
                document.activeElement === ref.current)
            ) {
              e.preventDefault();
              last?.focus();
            } else if (
              !e.shiftKey &&
              (document.activeElement === last ||
                document.activeElement === ref.current)
            ) {
              e.preventDefault();
              first?.focus();
            }
          }
        }}
      >
        <div className="modal-title">
          <h2>{title}</h2>
          <Button
            variant="ghost"
            onClick={onClose}
            aria-label={t("common.closeDialog")}
          >
            <X size={20} />
          </Button>
        </div>
        {children}
      </section>
    </div>
  );
}
export function Empty({
  title,
  description,
  action,
}: {
  title: string;
  description: string;
  action?: ReactNode;
}) {
  return (
    <div className="empty">
      <div className="empty-icon">
        <ServerIcon size={28} />
      </div>
      <h2>{title}</h2>
      <p>{description}</p>
      {action}
    </div>
  );
}
export function ErrorNotice({
  error,
  onClose,
}: {
  error: AppError;
  onClose: () => void;
}) {
  const { t } = useI18n();
  const key = errorMessageKey(error.code);
  return (
    <div className="notice error" role="alert">
      <div>
        <strong>{key ? t(key) : error.message}</strong>
        {(error.details || key) && (
          <details>
            <summary>{t("common.showDetails")}</summary>
            <pre>{key ? `${error.message}\n${error.details}`.trim() : error.details}</pre>
          </details>
        )}
      </div>
      <button aria-label={t("common.dismissError")} onClick={onClose}>
        <X size={18} />
      </button>
    </div>
  );
}
export function GuideLink({
  children,
  onClick,
}: {
  children: ReactNode;
  onClick: () => void;
}) {
  return (
    <button className="text-link" onClick={onClick}>
      {children}
      <ArrowUpRight size={14} />
    </button>
  );
}
export function Step({
  n,
  label,
  active,
  done,
}: {
  n: number;
  label: string;
  active: boolean;
  done: boolean;
}) {
  return (
    <div className={`step ${active ? "current" : ""} ${done ? "done" : ""}`}>
      <span>{done ? <Check size={13} /> : n}</span>
      {label}
    </div>
  );
}
export const bytes = (n: number) =>
  n >= 1024 ** 3
    ? `${(n / 1024 ** 3).toFixed(2)} GB`
    : n >= 1024 ** 2
      ? `${(n / 1024 ** 2).toFixed(1)} MB`
      : n >= 1024
        ? `${(n / 1024).toFixed(1)} KB`
        : `${n} B`;
export const date = (s: string) => new Date(s).toLocaleString();
