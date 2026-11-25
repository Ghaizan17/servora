/**
 * Accessible modal dialog.
 *
 * One dialog at a time, Escape closes it, focus is trapped inside and returned
 * to the element that opened it. The overlay is a sibling of the app shell so it
 * always paints above the sidebar.
 */

import { icons } from "../lib/icons.js";

let active = null;

function overlay() {
  let node = document.getElementById("servora-modal-layer");
  if (!node) {
    node = document.createElement("div");
    node.id = "servora-modal-layer";
    document.body.append(node);
  }
  return node;
}

/**
 * Open a dialog.
 *
 * @param {object} options
 * @param {string} options.title
 * @param {string} [options.description]
 * @param {string|HTMLElement} options.body
 * @param {Array<{label:string, tone?:string, primary?:boolean, onClick?:Function, type?:string, form?:string, disabled?:boolean}>} [options.actions]
 * @param {string} [options.width] tailwind max-width class
 * @param {Function} [options.onClose]
 * @returns {{root: HTMLElement, body: HTMLElement, close: Function, setBusy: Function, setError: Function}}
 */
export function openModal({
  title,
  description = "",
  body,
  actions = [],
  width = "max-w-2xl",
  onClose,
}) {
  closeModal();

  const previousFocus = document.activeElement;
  const root = document.createElement("div");
  root.className =
    "fixed inset-0 z-50 flex items-start justify-center overflow-y-auto bg-black/60 p-4 backdrop-blur-sm sm:p-8";
  root.setAttribute("role", "presentation");

  const panel = document.createElement("div");
  panel.className = `card relative my-auto w-full ${width} shadow-2xl`;
  panel.setAttribute("role", "dialog");
  panel.setAttribute("aria-modal", "true");
  panel.setAttribute("aria-label", title);

  const header = document.createElement("div");
  header.className = "flex items-start justify-between gap-4 border-b border-line px-6 py-4";
  header.innerHTML = `
    <div class="min-w-0">
      <h2 class="text-base font-semibold text-ink">${title}</h2>
      ${description ? `<p class="mt-1 text-sm text-muted">${description}</p>` : ""}
    </div>
    <button type="button" data-modal-close class="rounded-lg p-1.5 text-muted transition hover:bg-panel-soft hover:text-ink" aria-label="Close dialog">
      ${icons.x(16)}
    </button>
  `;

  const bodyNode = document.createElement("div");
  bodyNode.className = "px-6 py-5";
  if (typeof body === "string") bodyNode.innerHTML = body;
  else bodyNode.append(body);

  const errorBox = document.createElement("div");
  errorBox.className = "hidden mx-6 mb-1 rounded-lg border border-err/40 bg-err/10 px-3 py-2 text-sm text-err";
  errorBox.setAttribute("role", "alert");

  const footer = document.createElement("div");
  footer.className = "flex flex-wrap items-center justify-end gap-2 border-t border-line px-6 py-4";

  const buttons = [];
  for (const action of actions) {
    const button = document.createElement("button");
    button.className =
      action.tone === "danger"
        ? "btn btn-danger"
        : action.primary
          ? "btn btn-primary"
          : "btn";
    button.type = action.type ?? "button";
    if (action.form) button.setAttribute("form", action.form);
    button.textContent = action.label;
    button.disabled = Boolean(action.disabled);
    if (action.onClick) button.addEventListener("click", () => action.onClick(controller));
    buttons.push(button);
    footer.append(button);
  }

  panel.append(header, errorBox, bodyNode, footer);
  root.append(panel);
  overlay().append(root);

  const controller = {
    root,
    panel,
    body: bodyNode,
    error: errorBox,
    buttons,
    close: () => closeModal(),
    setBusy(busy, label = "Working…") {
      for (const button of buttons) {
        if (busy) {
          button.dataset.idleLabel = button.textContent;
          if (button.dataset.role === "primary") button.textContent = label;
          button.disabled = true;
        } else {
          if (button.dataset.idleLabel) button.textContent = button.dataset.idleLabel;
          button.disabled = false;
        }
      }
      panel.dataset.busy = busy ? "true" : "false";
    },
    setError(message, hint = "") {
      if (!message) {
        errorBox.classList.add("hidden");
        errorBox.innerHTML = "";
        return;
      }
      errorBox.innerHTML = `<p class="font-medium">${escapeHtml(message)}</p>${
        hint ? `<p class="mt-1 text-xs text-err/80">${escapeHtml(hint)}</p>` : ""
      }`;
      errorBox.classList.remove("hidden");
    },
  };

  for (const button of buttons) {
    if (button.dataset.idleLabel === undefined) button.dataset.idleLabel = button.textContent;
    if (button.classList.contains("btn-primary") || button.classList.contains("btn-danger")) {
      button.dataset.role = "primary";
    }
  }

  root.addEventListener("mousedown", (event) => {
    if (event.target === root) closeModal();
  });
  root.addEventListener("keydown", (event) => {
    if (event.key === "Escape") {
      event.stopPropagation();
      closeModal();
    }
    if (event.key === "Tab") {
      const focusable = panel.querySelectorAll(
        'button:not([disabled]), [href], input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])',
      );
      if (focusable.length === 0) return;
      const first = focusable[0];
      const last = focusable[focusable.length - 1];
      if (event.shiftKey && document.activeElement === first) {
        event.preventDefault();
        last.focus();
      } else if (!event.shiftKey && document.activeElement === last) {
        event.preventDefault();
        first.focus();
      }
    }
  });

  active = { root, onClose, previousFocus };
  const focusTarget = panel.querySelector("input, textarea, select, button[data-role='primary']") ?? panel;
  requestAnimationFrame(() => focusTarget.focus?.());
  return controller;
}

export function closeModal() {
  if (!active) return;
  const { root, onClose, previousFocus } = active;
  active = null;
  root.remove();
  onClose?.();
  previousFocus?.focus?.();
}

export function modalIsOpen() {
  return Boolean(active);
}

/**
 * Inline confirmation dialog. Resolves `true` when confirmed.
 */
export function confirmDialog({
  title,
  description,
  confirmLabel = "Confirm",
  tone = "danger",
  details = [],
}) {
  return new Promise((resolve) => {
    let settled = false;
    const finish = (value) => {
      if (settled) return;
      settled = true;
      resolve(value);
    };
    const detailsHtml = details.length
      ? `<ul class="mt-3 space-y-1.5 text-sm text-muted">${details
          .map((line) => `<li class="flex gap-2"><span aria-hidden="true">•</span><span>${line}</span></li>`)
          .join("")}</ul>`
      : "";
    openModal({
      title,
      description,
      body: `<div class="text-sm text-muted">${detailsHtml}</div>`,
      width: "max-w-lg",
      actions: [
        { label: "Cancel", onClick: (modal) => { modal.close(); finish(false); } },
        { label: confirmLabel, tone, onClick: (modal) => { modal.close(); finish(true); } },
      ],
      onClose: () => finish(false),
    });
  });
}

export function escapeHtml(value) {
  return String(value ?? "")
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;");
}
