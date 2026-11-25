/** Shared UI pieces used by more than one page. */
import { escapeHtml } from "../lib/format.js";

/**
 * A labelled switch rendered as a real button with `role="switch"`.
 */
export function switchRow({ id, label, description, checked, onToggle }) {
  const wrapper = document.createElement("div");
  wrapper.className = "flex items-start justify-between gap-6 py-3";
  wrapper.innerHTML = `
    <div class="min-w-0">
      <label class="text-[13px] font-medium text-ink" for="${id}">${escapeHtml(label)}</label>
      ${description ? `<p class="mt-0.5 text-[12px] leading-snug text-muted">${escapeHtml(description)}</p>` : ""}
    </div>
    <button id="${id}" class="switch mt-0.5" role="switch" aria-checked="${checked ? "true" : "false"}" aria-label="${escapeHtml(label)}">
      <span class="switch-knob"></span>
    </button>
  `;
  const button = wrapper.querySelector("button");
  button.addEventListener("click", () => {
    const next = button.getAttribute("aria-checked") !== "true";
    button.setAttribute("aria-checked", String(next));
    onToggle(next);
  });
  return wrapper;
}

/** Section heading with optional trailing action slot. */
export function sectionHeader({ title, description, action }) {
  const header = document.createElement("div");
  header.className = "flex items-start justify-between gap-4";
  header.innerHTML = `
    <div>
      <h2 class="text-[13px] font-semibold tracking-wide text-ink uppercase">${escapeHtml(title)}</h2>
      ${description ? `<p class="mt-1 text-[12px] text-muted">${escapeHtml(description)}</p>` : ""}
    </div>
    <div class="flex items-center gap-2">${action ?? ""}</div>
  `;
  return header;
}

/** Escape text, then render code spans so error hints stay readable. */
export function richText(value) {
  return escapeHtml(value ?? "").replace(
    /`([^`]+)`/g,
    '<code class="rounded bg-panel-soft px-1 py-0.5 font-mono text-[11px] text-ink-soft">$1</code>',
  );
}

/** Empty / error placeholder used by the log viewer. */
export function placeholder({ icon = "•", title, message, tone = "muted", action }) {
  return `
    <div class="flex flex-col items-center justify-center gap-2 px-6 py-14 text-center">
      <span class="text-2xl ${tone === "error" ? "text-err" : "text-muted"}">${icon}</span>
      <p class="text-[13px] font-semibold text-ink">${escapeHtml(title)}</p>
      <p class="max-w-md text-[12px] leading-relaxed text-muted">${richText(message)}</p>
      ${action ? `<div class="mt-2">${action}</div>` : ""}
    </div>
  `;
}

export function inlineSpinner(size = 14) {
  return `<svg class="animate-spin" width="${size}" height="${size}" viewBox="0 0 24 24" fill="none" aria-hidden="true">
    <circle cx="12" cy="12" r="9" stroke="currentColor" stroke-opacity="0.25" stroke-width="3"/>
    <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" stroke-width="3" stroke-linecap="round"/>
  </svg>`;
}
