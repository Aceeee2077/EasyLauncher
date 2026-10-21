import { getCurrentWindow } from "@tauri-apps/api/window";
import { desktop } from "./api";
import type { Settings } from "../types";
export async function applyAppearance(appearance: Settings["appearance"]) {
  document.documentElement.dataset.theme = appearance;
  if (desktop)
    await getCurrentWindow().setTheme(
      appearance === "system" ? null : appearance,
    );
}
/** Scales the glass surface alpha without fading text or icons. */
export function applyGlassOpacity(opacity: number) {
  const value = Number.isFinite(opacity) ? Math.min(1, Math.max(0.2, opacity)) : 1;
  document.documentElement.style.setProperty(
    "--glass-alpha",
    value.toFixed(3),
  );
}
