/**
 * Servora frontend entry point.
 *
 * Builds the application shell (sidebar + header + view host) and swaps the
 * three pages. Pages own their own state and polling; the shell only handles
 * navigation, the theme and the "outside the desktop shell" fallback.
 */
import "./styles/app.css";
import { api, isTauri } from "./lib/ipc.js";
import { icons } from "./lib/icons.js";
import { applyTheme, getSettings, loadSettings } from "./lib/store.js";
import { Dashboard } from "./pages/Dashboard.js";
import { Logs } from "./pages/Logs.js";
import { Settings } from "./pages/Settings.js";
import { toastError } from "./components/toast.js";
import { escapeHtml } from "./lib/format.js";

const ROUTES = [
  {
    id: "dashboard",
    label: "Dashboard",
    subtitle: "Control your local development stack.",
    icon: icons.dashboard,
    create: Dashboard,
  },
  {
    id: "logs",
    label: "Logs",
    subtitle: "Journal entries for the managed units.",
    icon: icons.logs,
    create: Logs,
  },
  {
    id: "settings",
    label: "Settings",
    subtitle: "Preferences, security and application info.",
    icon: icons.settings,
    create: Settings,
  },
];

function buildShell() {
  const app = document.getElementById("app");
  app.innerHTML = `
    <div class="flex h-screen overflow-hidden">
      <aside class="flex w-[212px] flex-none flex-col gap-4 border-r border-line bg-panel/50 px-3 py-4">
        <div class="flex items-center gap-2.5 px-1">
          <img src="/favicon.png" alt="" class="h-9 w-9 rounded-xl border border-line object-cover" />
          <div class="min-w-0">
            <p class="text-[15px] font-semibold leading-tight tracking-tight text-ink">Servora</p>
            <p class="truncate text-[10.5px] uppercase tracking-[0.14em] text-muted">Service Manager</p>
          </div>
        </div>

        <nav class="flex flex-col gap-1" aria-label="Main navigation">
          ${ROUTES.map(
            (route) => `
            <button class="nav-item" data-route="${route.id}">
              ${route.icon(16)}
              <span>${route.label}</span>
            </button>`,
          ).join("")}
        </nav>

        <div class="mt-auto flex flex-col gap-2 px-1 text-[11px] text-muted">
          <div class="flex items-center gap-2">
            <span data-role="connection-dot" class="dot"></span>
            <span data-role="connection">connecting…</span>
          </div>
          <p data-role="version">Servora</p>
        </div>
      </aside>

      <main class="flex min-w-0 flex-1 flex-col">
        <header class="flex items-center justify-between gap-4 border-b border-line px-5 py-3">
          <div class="min-w-0">
            <h1 data-role="title" class="text-[15px] font-semibold text-ink">Dashboard</h1>
            <p data-role="subtitle" class="truncate text-[12px] text-muted"></p>
          </div>
          <div class="flex items-center gap-2">
            <span data-role="header-badge" class="badge state-inactive"><span class="dot"></span><span data-role="header-state">checking</span></span>
          </div>
        </header>
        <div data-role="view" class="min-h-0 flex-1"></div>
      </main>
    </div>

    <div data-role="unsupported" class="hidden fixed inset-0 z-40 grid place-items-center bg-app px-6 text-center">
      <div class="max-w-md">
        <p class="text-[15px] font-semibold text-ink">Desktop shell required</p>
        <p data-role="unsupported-message" class="mt-2 text-[12.5px] leading-relaxed text-muted"></p>
      </div>
    </div>
  `;

  return {
    view: app.querySelector('[data-role="view"]'),
    title: app.querySelector('[data-role="title"]'),
    subtitle: app.querySelector('[data-role="subtitle"]'),
    headerBadge: app.querySelector('[data-role="header-badge"]'),
    headerState: app.querySelector('[data-role="header-state"]'),
    connection: app.querySelector('[data-role="connection"]'),
    connectionDot: app.querySelector('[data-role="connection-dot"]'),
    version: app.querySelector('[data-role="version"]'),
    unsupported: app.querySelector('[data-role="unsupported"]'),
    unsupportedMessage: app.querySelector('[data-role="unsupported-message"]'),
    navButtons: [...app.querySelectorAll("[data-route]")],
  };
}

async function main() {
  const shell = buildShell();
  let current = null;

  function activate(routeId) {
    const route = ROUTES.find((candidate) => candidate.id === routeId) ?? ROUTES[0];
    if (current?.route.id === route.id) return;

    current?.page.unmount();
    shell.view.replaceChildren();
    const page = route.create();
    shell.view.appendChild(page.element);
    current = { route, page };

    shell.title.textContent = route.label;
    shell.subtitle.textContent = route.subtitle;
    shell.navButtons.forEach((button) => {
      const active = button.dataset.route === route.id;
      if (active) button.setAttribute("aria-current", "page");
      else button.removeAttribute("aria-current");
    });
    page.mount();
    location.hash = route.id;
  }

  shell.navButtons.forEach((button) => {
    button.addEventListener("click", () => activate(button.dataset.route));
  });

  window.addEventListener("hashchange", () => {
    const id = location.hash.replace("#", "");
    if (id) activate(id);
  });

  await loadSettings();
  applyTheme(getSettings().theme);

  activate(ROUTES.some((r) => r.id === location.hash.replace("#", ""))
    ? location.hash.replace("#", "")
    : "dashboard");

  if (!isTauri()) {
    shell.unsupported.classList.remove("hidden");
    shell.unsupportedMessage.textContent =
      "This page is being served by Vite without Tauri, so the Rust backend is not available. Run `npm run app:dev` to start the full application.";
    return;
  }

  try {
    const info = await api.getAppInfo();
    shell.version.textContent = `v${info.version} · ${info.identifier}`;
  } catch (error) {
    toastError(error, "Could not read application info");
  }

  // Lightweight connection indicator driven by the same status command.
  async function updateConnection() {
    try {
      const response = await api.getServicesStatus();
      const reachable = !response.error;
      shell.connection.textContent = reachable
        ? `${response.running}/${response.total} running`
        : "systemd offline";
      shell.connectionDot.style.backgroundColor = reachable
        ? "var(--color-ok)"
        : "var(--color-err)";
      shell.headerBadge.className = `badge ${reachable ? "state-active" : "state-failed"}`;
      shell.headerState.textContent = reachable
        ? `${response.running}/${response.total} active`
        : "systemd offline";
    } catch {
      shell.connection.textContent = "backend offline";
      shell.connectionDot.style.backgroundColor = "var(--color-err)";
    }
  }

  updateConnection();
  window.setInterval(updateConnection, 5000);
}

main().catch((error) => {
  console.error(error);
  const app = document.getElementById("app");
  if (app) {
    app.innerHTML = `<div class="grid h-screen place-items-center px-8 text-center">
      <div class="max-w-md">
        <p class="text-[15px] font-semibold text-ink">Servora could not start</p>
        <p class="mt-2 text-[12.5px] text-muted">${escapeHtml(error?.message ?? String(error))}</p>
      </div>
    </div>`;
  }
});
