/**
 * Dashboard — the primary screen.
 *
 * Shows the three managed services with live systemd state, single-service
 * controls, batch controls and a rolling "last update" indicator. Status is
 * polled on a configurable interval; requests never overlap.
 */
import { api, onBatchProgress, onTrayRefresh, ServoraError } from "../lib/ipc.js";
import {
  clockTime,
  duration,
  escapeHtml,
  isRunning,
  isTransitioning,
  relativeTime,
  stateClass,
  stateLabel,
} from "../lib/format.js";
import { icons } from "../lib/icons.js";
import { getSettings, subscribe } from "../lib/store.js";
import { inlineSpinner, placeholder } from "../components/ui.js";
import { notify, toast, toastError } from "../components/toast.js";

const CARD_ORDER = ["httpd", "php-fpm", "mariadb"];

class ServiceCard {
  constructor(service) {
    this.service = service;
    this.status = null;
    this.busy = false;
    this.element = document.createElement("article");
    this.element.className =
      "card relative flex flex-col gap-4 p-4 transition-colors";
    this.element.dataset.service = service.id;
    this.element.innerHTML = `
      <header class="flex items-start justify-between gap-3">
        <div class="flex min-w-0 items-center gap-3">
          <span class="grid h-9 w-9 flex-none place-items-center rounded-xl border border-line bg-panel-soft text-accent">
            ${icons.server(17)}
          </span>
          <div class="min-w-0">
            <h3 class="truncate text-[14px] font-semibold text-ink">${escapeHtml(service.name)}</h3>
            <p class="truncate font-mono text-[11px] text-muted">${escapeHtml(service.unit)}</p>
          </div>
        </div>
        <span data-role="badge" class="badge state-inactive">
          <span class="dot"></span><span data-role="state">Unknown</span>
        </span>
      </header>

      <dl class="grid grid-cols-2 gap-2 text-[11.5px]">
        <div class="rounded-lg border border-line/70 bg-panel-soft/60 px-2.5 py-2">
          <dt class="text-muted">Sub-state</dt>
          <dd data-role="substate" class="mt-0.5 truncate font-mono text-[11.5px] text-ink-soft">—</dd>
        </div>
        <div class="rounded-lg border border-line/70 bg-panel-soft/60 px-2.5 py-2">
          <dt class="text-muted">Unit file</dt>
          <dd data-role="unitfile" class="mt-0.5 truncate font-mono text-[11.5px] text-ink-soft">—</dd>
        </div>
      </dl>

      <p data-role="notice" class="hidden rounded-lg border border-err/40 bg-err/10 px-2.5 py-2 text-[11.5px] leading-snug text-err"></p>

      <footer class="mt-auto flex items-center justify-between gap-2 pt-1">
        <button data-action="toggle" class="btn min-w-[104px] btn-primary" aria-pressed="false">
          <span data-role="toggle-icon">${icons.power(14)}</span>
          <span data-role="toggle-label">Turn ON</span>
        </button>
        <button data-action="restart" class="btn btn-ghost" title="Restart ${escapeHtml(service.name)}">
          ${icons.restart(14)}<span class="hidden sm:inline">Restart</span>
        </button>
      </footer>
      <div data-role="overlay" class="pointer-events-none absolute inset-0 hidden items-center justify-center rounded-[16px] bg-app/50 backdrop-blur-[1px]"></div>
    `;

    this.refs = {
      badge: this.element.querySelector('[data-role="badge"]'),
      state: this.element.querySelector('[data-role="state"]'),
      substate: this.element.querySelector('[data-role="substate"]'),
      unitfile: this.element.querySelector('[data-role="unitfile"]'),
      notice: this.element.querySelector('[data-role="notice"]'),
      toggle: this.element.querySelector('[data-action="toggle"]'),
      toggleLabel: this.element.querySelector('[data-role="toggle-label"]'),
      toggleIcon: this.element.querySelector('[data-role="toggle-icon"]'),
      restart: this.element.querySelector('[data-action="restart"]'),
      overlay: this.element.querySelector('[data-role="overlay"]'),
    };
  }

  setHandlers({ onToggle, onRestart }) {
    this.refs.toggle.addEventListener("click", () => onToggle(this.service.id));
    this.refs.restart.addEventListener("click", () => onRestart(this.service.id));
  }

  update(status) {
    this.status = status;
    const state = status?.state ?? "unknown";
    this.refs.badge.className = `badge ${stateClass(state)}`;
    this.refs.state.textContent = stateLabel(state);
    this.refs.substate.textContent = status?.subState || "—";
    this.refs.unitfile.textContent = status?.unitFileState || "—";

    const running = isRunning(state);
    this.refs.toggleLabel.textContent = running ? "Turn OFF" : "Turn ON";
    this.refs.toggleIcon.innerHTML = running ? icons.stop(13) : icons.power(14);
    this.refs.toggle.classList.toggle("btn-primary", !running);
    this.refs.toggle.classList.toggle("btn-danger", running);
    this.refs.toggle.setAttribute("aria-pressed", String(running));
    this.refs.toggle.setAttribute(
      "aria-label",
      running ? `Stop ${this.service.name}` : `Start ${this.service.name}`,
    );

    if (status && status.installed === false) {
      this.showNotice(
        `${this.service.unit} is not installed. Install the package to manage it.`,
      );
    } else if (state === "failed") {
      this.showNotice(
        "The unit is in a failed state. Restart it, then check the Logs page.",
      );
    } else if (status?.masked) {
      this.showNotice("The unit is masked, so systemd refuses to start it.");
    } else {
      this.hideNotice();
    }

    this.setBusy(this.busy || isTransitioning(state));
  }

  showNotice(text) {
    this.refs.notice.textContent = text;
    this.refs.notice.classList.remove("hidden");
  }

  hideNotice() {
    this.refs.notice.classList.add("hidden");
  }

  setBusy(busy) {
    this.busy = busy;
    this.refs.toggle.disabled = busy;
    this.refs.restart.disabled = busy;
    this.refs.overlay.classList.toggle("hidden", !busy);
    this.refs.overlay.classList.toggle("flex", busy);
    this.refs.overlay.innerHTML = busy
      ? `<span class="flex items-center gap-2 rounded-lg border border-line bg-panel px-3 py-1.5 text-[11.5px] font-medium text-ink-soft">${inlineSpinner(13)} working…</span>`
      : "";
  }
}

export function Dashboard() {
  const element = document.createElement("div");
  element.className = "flex h-full flex-col gap-4 overflow-y-auto px-5 py-4";
  element.innerHTML = `
    <section class="card flex flex-wrap items-center justify-between gap-4 px-5 py-4">
      <div class="flex items-center gap-4">
        <span data-role="overall-dot" class="grid h-11 w-11 place-items-center rounded-2xl border border-line bg-panel-soft text-muted">
          ${icons.shield(20)}
        </span>
        <div>
          <div class="flex items-center gap-2">
            <h2 data-role="overall" class="text-[15px] font-semibold text-ink">Checking services…</h2>
            <span data-role="overall-badge" class="badge state-inactive"><span class="dot"></span><span data-role="overall-count">—</span></span>
          </div>
          <p class="mt-0.5 text-[12px] text-muted">
            <span data-role="updated">never</span>
            <span class="text-line-strong"> · </span>
            <span data-role="autorefresh">auto refresh</span>
          </p>
        </div>
      </div>

      <div class="flex flex-wrap items-center gap-2">
        <button data-batch="start" class="btn btn-primary">${icons.play(12)}<span>Start All</span></button>
        <button data-batch="stop" class="btn">${icons.stop(12)}<span>Stop All</span></button>
        <button data-batch="restart" class="btn">${icons.restart(13)}<span>Restart All</span></button>
        <button data-role="refresh" class="btn btn-ghost" title="Refresh now">${icons.refresh(14)}</button>
      </div>
    </section>

    <section data-role="degraded" class="hidden"></section>

    <section data-role="batch-progress" class="hidden card-quiet px-4 py-3"></section>

    <section data-role="cards" class="grid gap-4 md:grid-cols-2 lg:grid-cols-3"></section>

    <p class="px-1 pb-1 text-[11.5px] leading-relaxed text-muted">
      Service actions are sent to systemd over the system bus and authorized by Polkit.
      Servora never runs as root and never stores a password.
    </p>
  `;

  const refs = {
    overall: element.querySelector('[data-role="overall"]'),
    overallDot: element.querySelector('[data-role="overall-dot"]'),
    overallBadge: element.querySelector('[data-role="overall-badge"]'),
    overallCount: element.querySelector('[data-role="overall-count"]'),
    updated: element.querySelector('[data-role="updated"]'),
    autorefresh: element.querySelector('[data-role="autorefresh"]'),
    refresh: element.querySelector('[data-role="refresh"]'),
    degraded: element.querySelector('[data-role="degraded"]'),
    progress: element.querySelector('[data-role="batch-progress"]'),
    cards: element.querySelector('[data-role="cards"]'),
  };

  const cards = new Map();
  for (const id of CARD_ORDER) {
    const meta = {
      id,
      name:
        id === "httpd" ? "Apache HTTP Server" : id === "php-fpm" ? "PHP-FPM" : "MariaDB",
      unit: `${id}.service`,
    };
    const card = new ServiceCard(meta);
    card.setHandlers({
      onToggle: (service) => toggleService(service),
      onRestart: (service) => restartService(service),
    });
    cards.set(id, card);
    refs.cards.appendChild(card.element);
  }

  let polling = null;
  let batchRunning = false;
  let tick = null;
  let lastObserved = null;
  let pollTimer = null;
  const progressState = new Map();

  function setUpdatedAt(epochMs) {
    lastObserved = epochMs;
    refs.updated.textContent = `Updated ${clockTime(epochMs)} (${relativeTime(epochMs)})`;
  }

  function renderOverall(response) {
    const services = response?.services ?? [];
    const running = services.filter((s) => isRunning(s.state)).length;
    const total = services.length || 3;
    const allRunning = running === total;

    refs.overall.textContent = allRunning
      ? "All services running"
      : running === 0
        ? "All services stopped"
        : `${running} of ${total} services running`;
    refs.overallCount.textContent = `${running}/${total}`;
    refs.overallBadge.className = `badge ${allRunning ? "state-active" : running > 0 ? "state-pending" : "state-inactive"}`;
    refs.overallDot.className = `grid h-11 w-11 place-items-center rounded-2xl border bg-panel-soft ${allRunning ? "border-ok/40 text-ok" : running > 0 ? "border-warn/40 text-warn" : "border-line text-muted"}`;
    setUpdatedAt(response?.observedAtMs ?? Date.now());
  }

  function renderDegraded(error) {
    if (!error) {
      refs.degraded.className = "hidden";
      refs.degraded.innerHTML = "";
      return;
    }
    refs.degraded.className = "card border-err/40 px-4 py-3";
    refs.degraded.innerHTML = placeholder({
      icon: "⚠",
      tone: "error",
      title: "systemd is not reachable",
      message: error.message + (error.hint ? ` — ${error.hint}` : ""),
    });
  }

  async function refreshStatus({ silent = false } = {}) {
    if (polling) return polling;
    polling = (async () => {
      try {
        const response = await api.getServicesStatus();
        for (const status of response.services) {
          cards.get(status.id)?.update(status);
        }
        renderOverall(response);
        renderDegraded(response.error);
        if (!silent && response.error) {
          toastError(new ServoraError(response.error), "Status unavailable");
        }
      } catch (error) {
        renderDegraded({
          message: error.message,
          hint: error.hint,
        });
        if (!silent) toastError(error, "Could not read service status");
      } finally {
        polling = null;
      }
    })();
    return polling;
  }

  function startAutoRefresh() {
    stopAutoRefresh();
    const settings = getSettings();
    refs.autorefresh.textContent = settings.autoRefresh
      ? `auto refresh every ${(settings.refreshIntervalMs / 1000).toFixed(settings.refreshIntervalMs % 1000 ? 1 : 0)}s`
      : "auto refresh off";
    if (!settings.autoRefresh) return;
    pollTimer = window.setInterval(() => {
      if (document.hidden) return;
      refreshStatus({ silent: true });
    }, settings.refreshIntervalMs);
  }

  function stopAutoRefresh() {
    if (pollTimer) window.clearInterval(pollTimer);
    pollTimer = null;
  }

  async function performOperation(label, runner, serviceName) {
    try {
      const result = await runner();
      toast({
        tone: "success",
        title: `${serviceName} ${result.operation === "stop" ? "stopped" : result.operation === "restart" ? "restarted" : "started"}`,
        message: `${duration(result.durationMs)} · state: ${stateLabel(result.status?.state)}`,
        timeout: 4000,
      });
      const settings = getSettings();
      if (settings.showNotifications) {
        notify({
          title: "Servora",
          body: `${serviceName} ${result.operation}ed successfully`,
        });
      }
      cards.get(result.service)?.update(result.status);
      await refreshStatus({ silent: true });
    } catch (error) {
      toastError(error, `${label} failed`);
      await refreshStatus({ silent: true });
    }
  }

  async function toggleService(id) {
    const card = cards.get(id);
    if (!card || card.busy || batchRunning) return;
    const running = isRunning(card.status?.state);
    card.setBusy(true);
    const name = card.service.name;
    await performOperation(running ? "Stop" : "Start", () => {
      return running ? api.stopService(id) : api.startService(id);
    }, name);
    card.setBusy(false);
  }

  async function restartService(id) {
    const card = cards.get(id);
    if (!card || card.busy || batchRunning) return;
    card.setBusy(true);
    await performOperation("Restart", () => api.restartService(id), card.service.name);
    card.setBusy(false);
  }

  function setBatchUi(running) {
    batchRunning = running;
    element
      .querySelectorAll("[data-batch]")
      .forEach((button) => (button.disabled = running));
    if (!running) {
      window.setTimeout(() => {
        refs.progress.className = "hidden";
        refs.progress.innerHTML = "";
      }, 2500);
    }
  }

  function renderProgress(payload) {
    const { operation, completed, total } = payload;
    if (payload.result) progressState.set(payload.result.service, payload.result);
    refs.progress.className = "card-quiet px-4 py-3";
    const dots = CARD_ORDER.map((id) => {
      const result = progressState.get(id);
      const tone = !result
        ? "state-inactive"
        : result.success
          ? "state-active"
          : "state-failed";
      return `<span class="badge ${tone}"><span class="dot"></span>${id}</span>`;
    }).join(" ");
    refs.progress.innerHTML = `
      <div class="flex items-center justify-between gap-3 text-[12px]">
        <span class="flex items-center gap-2 font-medium text-ink-soft">
          ${inlineSpinner(13)} ${escapeHtml(operation)} all — ${completed}/${total}
        </span>
        <span class="flex flex-wrap items-center gap-1.5">${dots}</span>
      </div>
    `;
  }

  async function runBatch(operation) {
    if (batchRunning) return;
    setBatchUi(true);
    progressState.clear();
    cards.forEach((card) => card.setBusy(true));
    refs.progress.className = "card-quiet px-4 py-3";
    refs.progress.innerHTML = `<div class="flex items-center gap-2 text-[12px] text-ink-soft">${inlineSpinner(13)} ${escapeHtml(operation)} all — starting…</div>`;

    try {
      const result =
        operation === "start"
          ? await api.startAll()
          : operation === "stop"
            ? await api.stopAll()
            : await api.restartAll();

      const failed = result.results.filter((r) => !r.success);
      if (result.allSucceeded) {
        toast({
          tone: "success",
          title: `${operation[0].toUpperCase()}${operation.slice(1)} All complete`,
          message: result.succeeded + " services updated successfully",
        });
      } else {
        toast({
          tone: "warn",
          title: `${operation} all finished with problems`,
          message: failed
            .map((r) => `${r.name}: ${r.error?.message ?? "unknown error"}`)
            .join(" · "),
          timeout: 9000,
        });
      }
      const settings = getSettings();
      if (settings.showNotifications) {
        notify({
          title: `Servora — ${operation} all`,
          body: `${result.succeeded} of ${result.results.length} services updated`,
        });
      }
    } catch (error) {
      toastError(error, `Could not ${operation} all services`);
    } finally {
      await refreshStatus({ silent: true });
      cards.forEach((card) => card.setBusy(false));
      setBatchUi(false);
    }
  }

  refs.refresh.addEventListener("click", () => {
    refreshStatus();
  });
  element.querySelectorAll("[data-batch]").forEach((button) => {
    button.addEventListener("click", () => runBatch(button.dataset.batch));
  });

  const unsubscribers = [];

  return {
    element,
    mount() {
      startAutoRefresh();
      refreshStatus();
      unsubscribers.push(subscribe(() => startAutoRefresh()));
      unsubscribers.push(onBatchProgress(renderProgress));
      unsubscribers.push(onTrayRefresh(() => refreshStatus({ silent: true })));
      tick = window.setInterval(() => {
        if (lastObserved) setUpdatedAt(lastObserved);
      }, 1000);
    },
    unmount() {
      stopAutoRefresh();
      if (tick) window.clearInterval(tick);
      tick = null;
      unsubscribers.forEach((dispose) => dispose());
      unsubscribers.length = 0;
    },
  };
}
