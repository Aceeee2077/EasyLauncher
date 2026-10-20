import { useState, useCallback, useRef } from "react";
import type { AppError } from "../types";
export function useTask() {
  const [error, setError] = useState<AppError | null>(null);
  const [busy, setBusy] = useState<Set<string>>(new Set());
  const pending = useRef(new Set<string>());
  const run = useCallback(
    async <T>(key: string, fn: () => Promise<T>): Promise<T | undefined> => {
      if (pending.current.has(key)) return;
      pending.current.add(key);
      setBusy(new Set(pending.current));
      try {
        return await fn();
      } catch (e) {
        const err = e as Partial<AppError>;
        setError({
          code: err.code,
          message: err.message || String(e),
          details: err.details || "",
        });
      } finally {
        pending.current.delete(key);
        setBusy(new Set(pending.current));
      }
    },
    [],
  );
  return { error, setError, busy, run };
}
