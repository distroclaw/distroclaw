import { useEffect, useState } from "react";
import { api } from "./api";

export type ThemePref = "system" | "light" | "dark";

/** Dark unless the user picked something else. */
export const DEFAULT_THEME: ThemePref = "dark";
/** Copy of the saved choice, read synchronously at startup so the first paint is already right. */
const CACHE_KEY = "distroclaw.theme";

const isPref = (v: unknown): v is ThemePref => v === "light" || v === "dark" || v === "system";

function apply(pref: ThemePref) {
  const root = document.documentElement;
  if (pref === "system") root.removeAttribute("data-theme");
  else root.setAttribute("data-theme", pref);
}

function cached(): ThemePref {
  try {
    const v = localStorage.getItem(CACHE_KEY);
    return isPref(v) ? v : DEFAULT_THEME;
  } catch {
    return DEFAULT_THEME;
  }
}

function remember(pref: ThemePref) {
  try {
    localStorage.setItem(CACHE_KEY, pref);
  } catch {
    /* the saved setting still applies on the next launch */
  }
}

/** Call before the first render. */
export function bootTheme() {
  apply(cached());
}

/** Shows a theme without saving it (Settings previews a choice until Save). */
export function previewTheme(pref: ThemePref) {
  apply(pref);
}

export function useTheme() {
  const [pref, setPref] = useState<ThemePref>(cached);

  useEffect(() => {
    api
      .getSetting("theme")
      .then((v) => {
        const next = isPref(v) ? v : DEFAULT_THEME;
        setPref(next);
        apply(next);
        remember(next);
      })
      .catch(() => {});
  }, []);

  const change = (next: ThemePref) => {
    setPref(next);
    apply(next);
    remember(next);
    api.setSetting("theme", next).catch(() => {});
  };

  return [pref, change] as const;
}
