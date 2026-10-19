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
