/**
 * Tiny observable store for the things several pages need: settings and the
 * last known service snapshot. No framework, just callbacks.
 */
import { api } from "./ipc.js";

const listeners = new Set();

const state = {
  settings: {
    theme: "dark",
    autoRefresh: true,
    refreshIntervalMs: 3000,
    minimizeToTray: true,
    startMinimized: false,
    showNotifications: true,
    logLineLimit: 300,
  },
  loaded: false,
};

export function getSettings() {
  return { ...state.settings };
}

export function subscribe(listener) {
  listeners.add(listener);
  return () => listeners.delete(listener);
}

function emit() {
  for (const listener of listeners) listener(state);
}

/** Resolve the effective theme (System follows the desktop preference). */
export function effectiveTheme(theme) {
  if (theme === "system") {
    return window.matchMedia?.("(prefers-color-scheme: light)").matches
      ? "light"
      : "dark";
  }
  return theme === "light" ? "light" : "dark";
}

export function applyTheme(theme) {
  const resolved = effectiveTheme(theme);
  document.documentElement.dataset.theme = resolved;
  document.documentElement.style.colorScheme = resolved;
}

export async function loadSettings() {
  try {
    const settings = await api.getSettings();
    state.settings = { ...state.settings, ...settings };
    state.loaded = true;
    applyTheme(state.settings.theme);
  } catch (error) {
    console.warn("could not load settings", error);
  }
  emit();
  return getSettings();
}

export async function saveSettings(patch) {
  const next = { ...state.settings, ...patch };
  const saved = await api.saveSettings(next);
  state.settings = { ...state.settings, ...saved };
  state.loaded = true;
  applyTheme(state.settings.theme);
  emit();
  return getSettings();
}

// Follow the desktop when the user picked "System".
if (typeof window !== "undefined" && window.matchMedia) {
  const query = window.matchMedia("(prefers-color-scheme: light)");
  query.addEventListener?.("change", () => {
    if (state.settings.theme === "system") applyTheme("system");
  });
}
