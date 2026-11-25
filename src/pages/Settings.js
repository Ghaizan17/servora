/**
 * Settings page — appearance, behaviour and application information.
 *
 * Every control saves immediately (validated by Rust) and reverts on failure,
 * so the stored configuration is always the configuration in effect.
 */
import { api } from "../lib/ipc.js";
import { escapeHtml } from "../lib/format.js";
import { icons } from "../lib/icons.js";
import { getSettings, saveSettings, subscribe } from "../lib/store.js";
import { switchRow } from "../components/ui.js";
import { toast, toastError } from "../components/toast.js";

const THEMES = [
  { id: "dark", label: "Dark" },
  { id: "light", label: "Light" },
  { id: "system", label: "System" },
];

export function Settings() {
  const element = document.createElement("div");
  element.className = "flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto";
  element.innerHTML = `
    <div class="grid gap-4 lg:grid-cols-2">
      <section class="card px-5 py-4">
        <h2 class="text-[13px] font-semibold tracking-wide text-ink uppercase">Appearance</h2>
        <p class="mt-1 text-[12px] text-muted">Servora is dark-first; pick the theme that fits your desktop.</p>
        <div class="mt-3 flex items-center justify-between gap-4 py-2">
          <div>
            <p class="text-[13px] font-medium text-ink">Theme</p>
            <p class="mt-0.5 text-[12px] text-muted">Applies to the whole application.</p>
          </div>
          <div class="segmented" role="group" aria-label="Theme">
            ${THEMES.map(
              (theme) =>
                `<button data-theme="${theme.id}" aria-pressed="${theme.id === "dark"}">${theme.label}</button>`,
            ).join("")}
          </div>
        </div>
      </section>

      <section class="card px-5 py-4">
        <h2 class="text-[13px] font-semibold tracking-wide text-ink uppercase">Status refresh</h2>
        <p class="mt-1 text-[12px] text-muted">How often the dashboard re-reads systemd state.</p>
        <div data-role="refresh-switch" class="mt-2 border-b border-line/60"></div>
        <div data-role="interval-row" class="flex items-center justify-between gap-6 py-3">
          <div class="min-w-0">
            <p class="text-[13px] font-medium text-ink">Refresh interval</p>
            <p class="mt-0.5 text-[12px] text-muted">Between 1 and 60 seconds.</p>
          </div>
          <div class="flex items-center gap-3">
            <input data-role="interval" type="range" min="1000" max="60000" step="500" class="w-32 accent-[var(--color-accent)]" aria-label="Refresh interval in milliseconds" />
            <span data-role="interval-value" class="w-12 text-right font-mono text-[12px] text-ink-soft">3.0s</span>
          </div>
        </div>
      </section>

      <section class="card px-5 py-4">
        <h2 class="text-[13px] font-semibold tracking-wide text-ink uppercase">Window &amp; notifications</h2>
        <p class="mt-1 text-[12px] text-muted">Closing the window never stops a service.</p>
        <div class="mt-2 divide-y divide-line/60">
          <div data-role="minimize-switch"></div>
          <div data-role="start-minimized-switch"></div>
          <div data-role="notifications-switch"></div>
        </div>
      </section>

      <section class="card px-5 py-4">
        <h2 class="text-[13px] font-semibold tracking-wide text-ink uppercase">Log viewer</h2>
        <p class="mt-1 text-[12px] text-muted">Default number of journal lines to request.</p>
        <div class="mt-3 flex items-center justify-between gap-6 py-2">
          <div class="min-w-0">
            <p class="text-[13px] font-medium text-ink">Line limit</p>
            <p class="mt-0.5 text-[12px] text-muted">Between 50 and 5000 lines.</p>
          </div>
          <div class="flex items-center gap-3">
            <input data-role="log-limit" type="range" min="50" max="5000" step="50" class="w-32 accent-[var(--color-accent)]" aria-label="Log line limit" />
            <span data-role="log-limit-value" class="w-12 text-right font-mono text-[12px] text-ink-soft">300</span>
          </div>
        </div>
      </section>
    </div>

    <section class="card px-5 py-4">
      <div class="flex items-center justify-between gap-3">
        <h2 class="text-[13px] font-semibold tracking-wide text-ink uppercase">Security</h2>
        <span data-role="auth-badge" class="badge"><span class="dot"></span>Polkit</span>
      </div>
      <div data-role="auth" class="mt-3 text-[12px] leading-relaxed text-ink-soft"></div>
    </section>

    <section class="card px-5 py-4">
      <h2 class="text-[13px] font-semibold tracking-wide text-ink uppercase">Application</h2>
      <dl data-role="info" class="mt-3 grid gap-x-6 gap-y-2 text-[12px] sm:grid-cols-2"></dl>
    </section>
  `;

  const refs = {
    refreshSwitch: element.querySelector('[data-role="refresh-switch"]'),
    intervalRow: element.querySelector('[data-role="interval-row"]'),
    interval: element.querySelector('[data-role="interval"]'),
    intervalValue: element.querySelector('[data-role="interval-value"]'),
    minimizeSwitch: element.querySelector('[data-role="minimize-switch"]'),
    startMinimizedSwitch: element.querySelector('[data-role="start-minimized-switch"]'),
    notificationsSwitch: element.querySelector('[data-role="notifications-switch"]'),
    logLimit: element.querySelector('[data-role="log-limit"]'),
    logLimitValue: element.querySelector('[data-role="log-limit-value"]'),
    auth: element.querySelector('[data-role="auth"]'),
    authBadge: element.querySelector('[data-role="auth-badge"]'),
    info: element.querySelector('[data-role="info"]'),
  };

  let settings = getSettings();
  let saving = false;

  function apply(next) {
    settings = next;
    renderControls();
  }

  async function persist(patch) {
    if (saving) return;
    saving = true;
    try {
      const saved = await saveSettings(patch);
      apply(saved);
      toast({ tone: "success", title: "Settings saved", timeout: 1600 });
    } catch (error) {
      toastError(error, "Could not save settings");
      apply(getSettings());
    } finally {
      saving = false;
    }
  }

  function renderControls() {
    element.querySelectorAll("[data-theme]").forEach((button) => {
      button.setAttribute(
        "aria-pressed",
        String(button.dataset.theme === settings.theme),
      );
    });
    refs.interval.value = String(settings.refreshIntervalMs);
    refs.intervalValue.textContent = `${(settings.refreshIntervalMs / 1000).toFixed(1)}s`;
    refs.interval.disabled = !settings.autoRefresh;
    refs.intervalRow.style.opacity = settings.autoRefresh ? "1" : "0.55";
    refs.logLimit.value = String(settings.logLineLimit);
    refs.logLimitValue.textContent = String(settings.logLineLimit);
  }

  function buildSwitches() {
    refs.refreshSwitch.replaceChildren(
      switchRow({
        id: "set-auto-refresh",
        label: "Auto refresh status",
        description: "Poll systemd while the dashboard is open.",
        checked: settings.autoRefresh,
        onToggle: (value) => persist({ autoRefresh: value }),
      }),
    );
    refs.minimizeSwitch.replaceChildren(
      switchRow({
        id: "set-minimize",
        label: "Minimize to tray on close",
        description: "Closing the window keeps Servora running in the tray.",
        checked: settings.minimizeToTray,
        onToggle: (value) => persist({ minimizeToTray: value }),
      }),
    );
    refs.startMinimizedSwitch.replaceChildren(
      switchRow({
        id: "set-start-minimized",
        label: "Start minimized",
        description: "Open Servora in the tray only.",
        checked: settings.startMinimized,
        onToggle: (value) => persist({ startMinimized: value }),
      }),
    );
    refs.notificationsSwitch.replaceChildren(
      switchRow({
        id: "set-notifications",
        label: "Desktop notifications",
        description: "Show a notification when a service changes state.",
        checked: settings.showNotifications,
        onToggle: (value) => persist({ showNotifications: value }),
      }),
    );
  }

  element.querySelectorAll("[data-theme]").forEach((button) => {
    button.addEventListener("click", () => persist({ theme: button.dataset.theme }));
  });

  refs.interval.addEventListener("change", () => {
    persist({ refreshIntervalMs: Number(refs.interval.value) });
  });
  refs.interval.addEventListener("input", () => {
    refs.intervalValue.textContent = `${(Number(refs.interval.value) / 1000).toFixed(1)}s`;
  });
  refs.logLimit.addEventListener("change", () => {
    persist({ logLineLimit: Number(refs.logLimit.value) });
  });
  refs.logLimit.addEventListener("input", () => {
    refs.logLimitValue.textContent = refs.logLimit.value;
  });

  async function loadInfo() {
    try {
      const [info, auth] = await Promise.all([
        api.getAppInfo(),
        api.getAuthorizationInfo(),
      ]);
      refs.authBadge.className = "badge state-active";
      refs.authBadge.innerHTML = `<span class="dot"></span>${escapeHtml(auth.mechanism)}`;
      refs.auth.innerHTML = `
        <p class="text-ink">${icons.shield(14)} <span class="ml-1">Polkit action <code class="font-mono text-[11.5px] text-accent">${escapeHtml(auth.actionId)}</code></span></p>
        <ul class="mt-2 space-y-1 text-muted">
          ${auth.notes.map((note) => `<li class="flex gap-2"><span class="text-line-strong">•</span><span>${escapeHtml(note)}</span></li>`).join("")}
        </ul>
        ${auth.runsAsRoot ? `<p class="mt-2 text-err">Servora is running as root. Start it as your desktop user instead.</p>` : ""}
      `;

      const rows = [
        ["Version", info.version],
        ["Identifier", info.identifier],
        ["License", info.license],
        ["Managed units", info.managedUnits.join(", ")],
        ["Configuration", info.configPath],
        ["Application log", info.logPath],
      ];
      refs.info.innerHTML = rows
        .map(
          ([label, value]) => `
        <div class="flex flex-col gap-0.5">
          <dt class="text-muted">${escapeHtml(label)}</dt>
          <dd class="break-all font-mono text-[11.5px] text-ink-soft">${escapeHtml(value)}</dd>
        </div>`,
        )
        .join("");
    } catch (error) {
      refs.auth.innerHTML = `<p class="text-err">${escapeHtml(error.message)}</p>`;
    }
  }

  buildSwitches();
  renderControls();

  return {
    element,
    mount() {
      loadInfo();
      const unsubscribe = subscribe((next) => {
        apply(next.settings);
        buildSwitches();
      });
      element._unsubscribeSettings = unsubscribe;
    },
    unmount() {
      element._unsubscribeSettings?.();
      element._unsubscribeSettings = null;
    },
  };
}
