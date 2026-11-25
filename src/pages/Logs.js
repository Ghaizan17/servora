/**
 * Logs page — journal entries for the three managed units via `journalctl`.
 */
import { api } from "../lib/ipc.js";
import { escapeHtml, relativeTime } from "../lib/format.js";
import { icons } from "../lib/icons.js";
import { getSettings } from "../lib/store.js";
import { inlineSpinner, placeholder } from "../components/ui.js";
import { toast, toastError } from "../components/toast.js";

const SERVICES = [
  { id: "httpd", label: "HTTPD" },
  { id: "php-fpm", label: "PHP-FPM" },
  { id: "mariadb", label: "MariaDB" },
];

const LEVELS = [
  { id: "all", label: "All" },
  { id: "errors", label: "Errors" },
  { id: "warnings", label: "Warnings" },
];

const LIMITS = [100, 300, 500, 1000, 2000];

export function Logs() {
  const element = document.createElement("div");
  element.className = "flex h-full min-h-0 flex-col gap-3 px-5 py-4";
  element.innerHTML = `
    <section class="card flex flex-wrap items-center justify-between gap-3 px-4 py-3">
      <div class="flex flex-wrap items-center gap-3">
        <div class="segmented" role="tablist" aria-label="Service">
          ${SERVICES.map(
            (service) =>
              `<button data-service="${service.id}" role="tab" aria-pressed="${service.id === "httpd"}">${service.label}</button>`,
          ).join("")}
        </div>
        <div class="segmented" aria-label="Log level">
          ${LEVELS.map(
            (level) =>
              `<button data-level="${level.id}" aria-pressed="${level.id === "all"}">${level.label}</button>`,
          ).join("")}
        </div>
        <label class="flex items-center gap-2 text-[12px] text-muted">
          Lines
          <select data-role="limit" class="field py-1 pr-6">
            ${LIMITS.map((limit) => `<option value="${limit}">${limit}</option>`).join("")}
          </select>
        </label>
      </div>

      <div class="flex items-center gap-2">
        <button data-role="autorefresh" class="btn" aria-pressed="false">${icons.refresh(13)}<span>Auto</span></button>
        <button data-role="refresh" class="btn">${icons.refresh(13)}<span>Refresh</span></button>
        <button data-role="copy" class="btn btn-ghost" title="Copy visible logs">${icons.copy(13)}</button>
      </div>
    </section>

    <section class="card flex min-h-0 flex-1 flex-col overflow-hidden">
      <header class="flex items-center justify-between gap-3 border-b border-line px-4 py-2 text-[11.5px] text-muted">
        <span data-role="summary">No logs loaded yet</span>
        <span data-role="updated">—</span>
      </header>
      <div data-role="body" class="scroll-area min-h-0 flex-1"></div>
    </section>
  `;

  const refs = {
    body: element.querySelector('[data-role="body"]'),
    summary: element.querySelector('[data-role="summary"]'),
    updated: element.querySelector('[data-role="updated"]'),
    limit: element.querySelector('[data-role="limit"]'),
    refresh: element.querySelector('[data-role="refresh"]'),
    copy: element.querySelector('[data-role="copy"]'),
    autorefresh: element.querySelector('[data-role="autorefresh"]'),
  };

  const state = {
    service: "httpd",
    level: "all",
    limit: LIMITS.includes(getSettings().logLineLimit)
      ? getSettings().logLineLimit
      : 300,
    autoRefresh: false,
    loading: false,
    error: null,
    data: null,
  };

  refs.limit.value = String(state.limit);
  render();

  function summaryText() {
    if (state.error) return "Could not read the journal";
    if (!state.data) return "No logs loaded yet";
    const { count, requestedLines, truncated } = state.data;
    const base = `${count} line${count === 1 ? "" : "s"} (limit ${requestedLines})`;
    return truncated ? `${base} · output truncated` : base;
  }

  function highlight(message) {
    const escaped = escapeHtml(message);
    return escaped.replace(
      /\b(error|failed|failure|fatal|critical|warning|refused|denied)\b/gi,
      (match) => `<span class="text-err font-medium">${match}</span>`,
    );
  }

  function render() {
    refs.summary.textContent = summaryText();
    refs.updated.textContent = state.data?.retrievedAtMs
      ? `fetched ${relativeTime(state.data.retrievedAtMs)}`
      : "—";
    refs.autorefresh.setAttribute("aria-pressed", String(state.autoRefresh));
    refs.autorefresh.classList.toggle("btn-primary", state.autoRefresh);
    refs.refresh.disabled = state.loading;

    if (state.loading && !state.data) {
      refs.body.innerHTML = `<div class="flex items-center gap-2 px-4 py-6 text-[12px] text-muted">${inlineSpinner(13)} reading journal…</div>`;
      return;
    }

    if (state.error) {
      refs.body.innerHTML = placeholder({
        icon: "⚠",
        tone: "error",
        title: "The journal could not be read",
        message:
          state.error.message + (state.error.hint ? ` — ${state.error.hint}` : ""),
        action: `<button data-retry class="btn">${icons.refresh(13)}<span>Try again</span></button>`,
      });
      refs.body.querySelector("[data-retry]")?.addEventListener("click", () => load());
      return;
    }

    const entries = state.data?.entries ?? [];
    if (entries.length === 0) {
      refs.body.innerHTML = placeholder({
        icon: "○",
        title: "No matching journal entries",
        message: `systemd has not written any ${
          state.level === "all" ? "" : state.level === "errors" ? "error " : "warning "
        }entries for ${state.service}.service yet.`,
      });
      return;
    }

    const rows = entries
      .map((entry) => {
        const time = entry.timestamp
          ? escapeHtml(entry.timestamp.replace("T", " ").slice(0, 19))
          : "—";
        return `<div class="log-line">
          <span class="text-muted whitespace-nowrap">${time}</span>
          <span class="text-accent/80 whitespace-nowrap">${escapeHtml(entry.ident ?? "")}</span>
          <span class="break-words text-ink-soft">${highlight(entry.message)}</span>
        </div>`;
      })
      .join("");
    refs.body.innerHTML = rows;
    refs.body.scrollTop = refs.body.scrollHeight;
  }

  async function load({ silent = false } = {}) {
    if (state.loading) return;
    state.loading = true;
    state.error = null;
    render();
    try {
      state.data = await api.getServiceLogs({
        service: state.service,
        level: state.level,
        limit: state.limit,
      });
    } catch (error) {
      state.error = { message: error.message, hint: error.hint };
      if (!silent) toastError(error, "Could not load logs");
    } finally {
      state.loading = false;
      render();
    }
  }

  element.querySelectorAll("[data-service]").forEach((button) => {
    button.addEventListener("click", () => {
      state.service = button.dataset.service;
      state.data = null;
      element.querySelectorAll("[data-service]").forEach((other) => {
        const active = other.dataset.service === state.service;
        other.setAttribute("aria-pressed", String(active));
        other.setAttribute("aria-selected", String(active));
      });
      load();
    });
  });

  element.querySelectorAll("[data-level]").forEach((button) => {
    button.addEventListener("click", () => {
      state.level = button.dataset.level;
      element.querySelectorAll("[data-level]").forEach((other) => {
        other.setAttribute(
          "aria-pressed",
          String(other.dataset.level === state.level),
        );
      });
      load();
    });
  });

  refs.limit.addEventListener("change", () => {
    state.limit = Number(refs.limit.value) || 300;
    load();
  });

  refs.refresh.addEventListener("click", () => load());

  refs.autorefresh.addEventListener("click", () => {
    state.autoRefresh = !state.autoRefresh;
    startTimer();
    render();
  });

  refs.copy.addEventListener("click", async () => {
    const entries = state.data?.entries ?? [];
    if (entries.length === 0) {
      toast({ tone: "info", title: "Nothing to copy", message: "No log lines are loaded." });
      return;
    }
    const text = entries
      .map(
        (entry) =>
          `${entry.timestamp ?? ""} ${entry.ident ? `[${entry.ident}] ` : ""}${entry.message}`,
      )
      .join("\n");
    try {
      await navigator.clipboard.writeText(text);
      toast({
        tone: "success",
        title: "Logs copied",
        message: `${entries.length} lines copied to the clipboard`,
        timeout: 2600,
      });
    } catch {
      const area = document.createElement("textarea");
      area.value = text;
      document.body.appendChild(area);
      area.select();
      const ok = document.execCommand?.("copy");
      area.remove();
      toast(
        ok
          ? { tone: "success", title: "Logs copied", timeout: 2600 }
          : { tone: "error", title: "Clipboard unavailable", message: "Select the text and copy it manually." },
      );
    }
  });

  let timer = null;
  function startTimer() {
    if (timer) window.clearInterval(timer);
    timer = null;
    if (!state.autoRefresh) return;
    timer = window.setInterval(() => {
      if (!document.hidden) load({ silent: true });
    }, 5000);
  }

  return {
    element,
    mount() {
      load();
    },
    unmount() {
      if (timer) window.clearInterval(timer);
      timer = null;
    },
  };
}
