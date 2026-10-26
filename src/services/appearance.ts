import { convertFileSrc } from "@tauri-apps/api/core";
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
/** Backdrop blur in pixels; glass.css expands the layer so edges stay solid. */
export function applyBackgroundBlur(blur: number) {
  const value = Number.isFinite(blur) ? Math.min(40, Math.max(0, blur)) : 0;
  document.documentElement.style.setProperty(
    "--backdrop-blur",
    `${value.toFixed(1)}px`,
  );
}
/**
 * Points the backdrop at the wallpaper this app copied into its data folder.
 * Passing null restores the bundled landscape. The image is probed first so a
 * missing or broken file falls back silently instead of leaving a blank stage.
 */
let backgroundRequest = 0;
export function applyBackgroundImage(path: string | null): Promise<boolean> {
  const root = document.documentElement;
  const request = ++backgroundRequest;
  if (!path || !desktop) {
    root.style.removeProperty("--backdrop-artwork");
    return Promise.resolve(false);
  }
  const url = convertFileSrc(path).replaceAll('"', "%22");
  return new Promise((resolve) => {
    const image = new Image();
    image.onload = () => {
      // A newer selection finished first; do not overwrite it.
      if (request !== backgroundRequest) return resolve(false);
      root.style.setProperty("--backdrop-artwork", `url("${url}")`);
      resolve(true);
    };
    image.onerror = () => {
      if (request !== backgroundRequest) return resolve(false);
      root.style.removeProperty("--backdrop-artwork");
      resolve(false);
    };
    image.src = url;
  });
}
