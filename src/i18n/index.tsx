import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { messages, type MessageKey } from "./messages";

export type Language = "system" | "zh-CN" | "en";
export type Locale = "zh-CN" | "en";

const STORAGE_KEY = "minehost.language";

function stored(): Language {
  try {
    const value = localStorage.getItem(STORAGE_KEY);
    if (value === "system" || value === "zh-CN" || value === "en") return value;
  } catch {
    /* Storage may be unavailable in restricted webviews. */
  }
  return "zh-CN";
}

function systemLocale(): Locale {
  if (typeof navigator === "undefined") return "zh-CN";
  const list =
    navigator.languages && navigator.languages.length
      ? navigator.languages
      : [navigator.language];
  return list.some((l) => l.toLowerCase().startsWith("zh")) ? "zh-CN" : "en";
}

type I18n = {
  language: Language;
  locale: Locale;
  setLanguage: (language: Language) => void;
  t: (key: MessageKey, vars?: Record<string, string | number>) => string;
};

const I18nContext = createContext<I18n | null>(null);

export function LanguageProvider({ children }: { children: ReactNode }) {
  const [language, setLanguageState] = useState<Language>(stored);
  const [detected, setDetected] = useState<Locale>(systemLocale);
  useEffect(() => {
    const update = () => setDetected(systemLocale());
    window.addEventListener("languagechange", update);
    return () => window.removeEventListener("languagechange", update);
  }, []);
  const locale: Locale = language === "system" ? detected : language;
  useEffect(() => {
    document.documentElement.lang = locale;
    document.documentElement.dataset.locale = locale;
  }, [locale]);
  const setLanguage = useCallback((next: Language) => {
    setLanguageState(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      /* Preference still applies for this session. */
    }
  }, []);
  const t = useCallback(
    (key: MessageKey, vars?: Record<string, string | number>) => {
      const template = messages[locale][key] ?? messages["zh-CN"][key] ?? key;
      if (!vars) return template;
      return template.replace(/\{(\w+)\}/g, (match, name: string) =>
        vars[name] === undefined ? match : String(vars[name]),
      );
    },
    [locale],
  );
  const value = useMemo(
    () => ({ language, locale, setLanguage, t }),
    [language, locale, setLanguage, t],
  );
  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n() {
  const context = useContext(I18nContext);
  if (!context) throw new Error("useI18n must be used inside LanguageProvider");
  return context;
}

export type { MessageKey };
