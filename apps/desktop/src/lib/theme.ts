import { useEffect } from "react";
import { resolveTheme, type ThemePreference } from "@captureone/shared-types";

/** Applies the theme preference to <html data-theme>, following the OS when set to "system". */
export function useApplyTheme(pref: ThemePreference): void {
  useEffect(() => {
    const mq = typeof window.matchMedia === "function" ? window.matchMedia("(prefers-color-scheme: dark)") : null;
    const apply = () => {
      document.documentElement.dataset.theme = resolveTheme(pref, mq?.matches ?? false);
    };
    apply();
    mq?.addEventListener("change", apply);
    return () => mq?.removeEventListener("change", apply);
  }, [pref]);
}
