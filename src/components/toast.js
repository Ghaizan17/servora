/**
 * Toast notifications plus optional native desktop notifications.
 *
 * Both are purely presentational: they never change application state.
 */
import { escapeHtml } from "../lib/format.js";

let host = null;
const MAX_VISIBLE = 4;

function getHost() {
  if (!host) host = document.getElementById("toast-host");
  return host;
}

const TONES = {
  success: { icon: "✓", classes: "border-ok/40 text-ok" },
  error: { icon: "!", classes: "border-err/45 text-err" },
  info: { icon: "i", classes: "border-line-strong text-ink-soft" },
  warn: { icon: "!", classes: "border-warn/45 text-warn" },
};

export function toast({ tone = "info", title, message, hint, timeout = 5200 }) {
  const container = getHost();
  if (!container) return;

  while (container.children.length >= MAX_VISIBLE) {
    container.firstElementChild?.remove();
  }

  const style = TONES[tone] ?? TONES.info;
  const element = document.createElement("div");
  element.className = `toast card pointer-events-auto flex w-full max-w-md items-start gap-3 px-3.5 py-3 shadow-lg shadow-black/30 ${style.classes}`;
  element.setAttribute("role", tone === "error" ? "alert" : "status");
  element.innerHTML = `
    <span class="mt-0.5 grid h-5 w-5 flex-none place-items-center rounded-full border text-[11px] font-bold ${style.classes}">${style.icon}</span>
    <div class="min-w-0 flex-1">
      <p class="text-[13px] font-semibold text-ink">${escapeHtml(title)}</p>
      ${message ? `<p class="mt-0.5 text-[12px] leading-snug text-ink-soft">${escapeHtml(message)}</p>` : ""}
      ${hint ? `<p class="mt-1 text-[11.5px] leading-snug text-muted">${escapeHtml(hint)}</p>` : ""}
    </div>
    <button class="btn btn-ghost -mr-1 -mt-1 h-6 w-6 px-0 text-muted" aria-label="Dismiss notification">✕</button>
  `;

  const dismiss = () => element.remove();
  element.querySelector("button")?.addEventListener("click", dismiss);
  container.appendChild(element);
  if (timeout > 0) window.setTimeout(dismiss, timeout);
}

export function toastError(error, title = "Something went wrong") {
  toast({
    tone: "error",
    title,
    message: error?.message ?? String(error),
    hint: error?.hint ?? null,
    timeout: 9000,
  });
}

/** Native notification, used when the user enabled them in Settings. */
export async function notify({ title, body }) {
  try {
    const { isPermissionGranted, requestPermission, sendNotification } =
      await import("@tauri-apps/plugin-notification");
    let granted = await isPermissionGranted();
    if (!granted) {
      const permission = await requestPermission();
      granted = permission === "granted";
    }
    if (granted) sendNotification({ title, body });
  } catch {
    // Native notifications are best effort; the in-app toast already shows it.
  }
}
