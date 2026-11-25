/**
 * Websites — the Website Manager.
 *
 * Lists every website Servora manages, with search, filtering, sorting and the
 * create / edit / rename / remove workflows. All decisions (validation,
 * document root detection, Apache rendering, rollback) happen in the Rust core;
 * this page only collects input and reports what came back.
 */

import { api, ServoraError } from "../lib/ipc.js";
import { icons } from "../lib/icons.js";
import { escapeHtml, relativeTime } from "../lib/format.js";
import { toast, toastError } from "../components/toast.js";
import { inlineSpinner, placeholder } from "../components/ui.js";
import { confirmDialog, openModal } from "../components/modal.js";
import { createWebsiteForm } from "../components/websiteForm.js";
import { openMigrationDialog } from "../components/migrationDialog.js";

const STATUS_META = {
  live: { label: "Live", className: "state-active" },
  "hosts-missing": { label: "Hosts missing", className: "state-pending" },
  "config-missing": { label: "Config missing", className: "state-failed" },
  "config-mismatch": { label: "Config mismatch", className: "state-failed" },
  "project-missing": { label: "Project missing", className: "state-failed" },
  degraded: { label: "Needs attention", className: "state-pending" },
};

const SORTS = [
  { id: "name", label: "Name" },
  { id: "recent", label: "Recent" },
  { id: "type", label: "Type" },
];

const ORIGIN_LABELS = {
  created: "Created in Servora",
  importedSiteForge: "Imported from siteForge",
  importedLegacy: "Imported from a legacy VirtualHost",
  recovered: "Recovered from the live configuration",
};

function originLabel(origin) {
  return ORIGIN_LABELS[origin] ?? origin ?? "Created in Servora";
}

function originBadge(origin) {
  if (!origin || origin === "created") return "";
  const text =
    origin === "importedSiteForge" ? "siteForge" : origin === "recovered" ? "recovered" : "legacy";
  const title =
    origin === "recovered"
      ? "Rebuilt from its VirtualHost because the stored record was missing"
      : "Imported instead of created here";
  return `<span class="badge" title="${title}">${text}</span>`;
}

export function Websites() {
  const element = document.createElement("div");
  element.className = "flex min-h-0 flex-1 flex-col gap-4";

  element.innerHTML = `
    <div class="flex flex-wrap items-center gap-2">
      <div class="relative min-w-[180px] flex-1">
        <span class="pointer-events-none absolute top-1/2 left-3 -translate-y-1/2 text-muted">${icons.search(15)}</span>
        <input data-role="search" type="search" class="field w-full pl-9" placeholder="Search websites, domains or paths…" aria-label="Search websites" />
      </div>
      <div class="segmented" role="group" aria-label="Filter websites">
        <button data-filter="all" aria-pressed="true">All</button>
        <button data-filter="live" aria-pressed="false">Live</button>
        <button data-filter="issues" aria-pressed="false">Needs attention</button>
      </div>
      <div class="segmented" role="group" aria-label="Sort websites">
        ${SORTS.map(
          (sort, index) =>
            `<button data-sort="${sort.id}" aria-pressed="${index === 0}">${sort.label}</button>`,
        ).join("")}
      </div>
      <button data-role="refresh" class="btn" title="Reload the website list">${icons.refresh(14)}</button>
      <button data-role="import" class="btn" title="Import websites created by siteForge">
        ${icons.download(14)}<span class="hidden md:inline">Import</span>
      </button>
      <button data-role="create" class="btn btn-primary">${icons.plus(14)} New website</button>
    </div>

    <div data-role="summary" class="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11.5px] text-muted"></div>

    <div data-role="list" class="scroll-area min-h-0 flex-1 -mx-1 px-1"></div>
  `;

  const list = element.querySelector('[data-role="list"]');
  const search = element.querySelector('[data-role="search"]');
  const summary = element.querySelector('[data-role="summary"]');
  const refreshButton = element.querySelector('[data-role="refresh"]');

  let websites = [];
  let loading = true;
  let error = null;
  let query = "";
  let filter = "all";
  let sort = "name";

  function filtered() {
    const needle = query.trim().toLowerCase();
    let rows = websites.filter((website) => {
      if (filter === "live" && website.state !== "live") return false;
      if (filter === "issues" && website.state === "live") return false;
      if (!needle) return true;
      return [website.name, website.domain, website.projectPath, website.projectType]
        .filter(Boolean)
        .some((value) => String(value).toLowerCase().includes(needle));
    });
    rows = [...rows].sort((a, b) => {
      if (sort === "recent") return (b.updatedAtMs ?? 0) - (a.updatedAtMs ?? 0);
      if (sort === "type") return a.projectType.localeCompare(b.projectType) || a.name.localeCompare(b.name);
      return a.name.localeCompare(b.name);
    });
    return rows;
  }

  function statusBadge(state) {
    const meta = STATUS_META[state] ?? STATUS_META.degraded;
    return `<span class="badge ${meta.className}"><span class="dot"></span>${meta.label}</span>`;
  }

  function renderRow(website) {
    const row = document.createElement("article");
    row.className = "card group flex flex-wrap items-center gap-3 px-4 py-3";
    row.dataset.website = website.id;
    row.innerHTML = `
      <div class="grid h-9 w-9 flex-none place-items-center rounded-xl border border-line bg-panel-soft text-accent">
        ${icons.globe(17)}
      </div>
      <div class="min-w-0 flex-1">
        <div class="flex flex-wrap items-center gap-2">
          <h3 class="truncate text-[13.5px] font-semibold text-ink">${escapeHtml(website.name)}</h3>
          ${statusBadge(website.state)}
          <span class="badge">${escapeHtml(website.projectType)}</span>
          ${originBadge(website.origin)}
        </div>
        <div class="mt-0.5 flex flex-wrap items-center gap-x-3 gap-y-0.5 text-[11.5px] text-muted">
          <a href="${escapeHtml(website.url)}" data-role="open" class="font-mono text-accent hover:underline">${escapeHtml(website.domain)}</a>
          <span class="truncate font-mono" title="${escapeHtml(website.documentRoot)}">${escapeHtml(shortPath(website.documentRoot))}</span>
        </div>
      </div>
      <div class="ml-auto flex items-center gap-1.5">
        <button data-action="details" class="btn btn-ghost" title="Details">${icons.info(14)}</button>
        <button data-action="edit" class="btn btn-ghost" title="Edit website">${icons.pencil(14)}</button>
        <button data-action="remove" class="btn btn-ghost btn-danger" title="Remove website">${icons.trash(14)}</button>
      </div>
    `;

    row.querySelector('[data-action="details"]').addEventListener("click", () => showDetails(website));
    row.querySelector('[data-action="edit"]').addEventListener("click", () => showEdit(website));
    row.querySelector('[data-action="remove"]').addEventListener("click", () => showRemove(website));
    const open = row.querySelector('[data-role="open"]');
    open.addEventListener("click", (event) => {
      event.preventDefault();
      openUrl(website.url);
    });
    return row;
  }

  function render() {
    list.innerHTML = "";
    list.className = "scroll-area min-h-0 flex-1 -mx-1 px-1";

    if (loading) {
      list.className = "scroll-area min-h-0 flex-1 flex flex-col justify-center";
      list.innerHTML = `<div class="flex items-center justify-center gap-2 py-16 text-[12.5px] text-muted">${inlineSpinner(16)} Loading websites…</div>`;
      return;
    }

    if (error) {
      list.className = "scroll-area min-h-0 flex-1 flex flex-col justify-center";
      list.append(
        placeholder({
          icon: icons.alert(20),
          title: "The website list could not be read",
          message: error.message,
          tone: "danger",
          action: { label: "Try again", onClick: load },
        }),
      );
      if (error.hint) {
        const hint = document.createElement("p");
        hint.className = "mt-2 text-center text-[11.5px] text-muted";
        hint.textContent = error.hint;
        list.append(hint);
      }
      return;
    }

    if (websites.length === 0) {
      list.className = "scroll-area min-h-0 flex-1 flex flex-col justify-center";
      list.append(
        placeholder({
          icon: icons.globe(22),
          title: "No websites yet",
          message:
            "Add a project directory and Servora writes the Apache VirtualHost, the hosts entry, ACLs and SELinux rules for you.",
          tone: "muted",
          action: { label: "Create a website", onClick: () => showCreate() },
        }),
      );
      return;
    }

    const rows = filtered();
    if (rows.length === 0) {
      list.className = "scroll-area min-h-0 flex-1 flex flex-col justify-center";
      list.append(
        placeholder({
          icon: icons.search(22),
          title: "Nothing matches your filters",
          message: "Clear the search box or switch back to “All”.",
          tone: "muted",
          action: {
            label: "Reset filters",
            onClick: () => {
              search.value = "";
              query = "";
              filter = "all";
              element.querySelectorAll("[data-filter]").forEach((button) => {
                button.setAttribute("aria-pressed", String(button.dataset.filter === "all"));
              });
              render();
            },
          },
        }),
      );
      return;
    }

    const container = document.createElement("div");
    container.className = "flex flex-col gap-2";
    for (const website of rows) container.append(renderRow(website));
    list.append(container);
  }

  function renderSummary() {
    const live = websites.filter((website) => website.state === "live").length;
    const issues = websites.length - live;
    summary.innerHTML = `
      <span>${websites.length} website${websites.length === 1 ? "" : "s"}</span>
      <span class="text-ok">${live} served</span>
      ${issues > 0 ? `<span class="text-warn">${issues} need attention</span>` : ""}
    `;
  }

  async function load() {
    try {
      websites = await api.getWebsites();
      error = null;
    } catch (failure) {
      error = failure;
      websites = [];
    } finally {
      loading = false;
      renderSummary();
      render();
    }
  }

  // ------------------------------------------------------------- workflows

  function showCreate() {
    const form = createWebsiteForm({ mode: "create" });
    openModal({
      title: "New website",
      description:
        "Servora detects the project type and document root, then writes the VirtualHost, hosts entry and permissions.",
      body: form.element,
      width: "max-w-3xl",
      actions: [
        { label: "Cancel", onClick: (modal) => modal.close() },
        {
          label: "Create website",
          primary: true,
          onClick: async (modal) => {
            const problem = form.validate();
            if (problem) {
              modal.setError(problem);
              return;
            }
            modal.setError("");
            modal.setBusy(true, "Writing configuration…");
            try {
              const report = await api.createWebsite(form.payload());
              modal.close();
              announceResult(report, "Website created");
              await load();
            } catch (failure) {
              modal.setBusy(false);
              reportFailure(modal, failure);
            }
          },
        },
      ],
    });
  }

  function showEdit(website) {
    const form = createWebsiteForm({ mode: "edit", initial: website });
    openModal({
      title: `Edit ${website.name}`,
      description: "Changes are applied transactionally: any failing step rolls the others back.",
      body: form.element,
      width: "max-w-3xl",
      actions: [
        { label: "Cancel", onClick: (modal) => modal.close() },
        {
          label: "Save changes",
          primary: true,
          onClick: async (modal) => {
            const problem = form.validate();
            if (problem) {
              modal.setError(problem);
              return;
            }
            modal.setError("");
            modal.setBusy(true, "Applying changes…");
            try {
              const report = await api.updateWebsite(website.id, form.payload());
              modal.close();
              announceResult(report, "Website updated");
              await load();
            } catch (failure) {
              modal.setBusy(false);
              reportFailure(modal, failure);
            }
          },
        },
      ],
    });
  }

  async function showDetails(website) {
    const modal = openModal({
      title: website.name,
      description: website.domain,
      width: "max-w-2xl",
      body: `<div class="flex items-center gap-2 text-[12.5px] text-muted">${inlineSpinner(14)} Loading details…</div>`,
      actions: [
        { label: "Close", onClick: (controller) => controller.close() },
        {
          label: "Rename domain",
          onClick: (controller) => {
            controller.close();
            showRename(website);
          },
        },
        {
          label: "Open in browser",
          primary: true,
          onClick: () => openUrl(website.url),
        },
      ],
    });

    let vhost = null;
    let history = [];
    try {
      vhost = await api.getVirtualhostStatus(website.domain);
      history = await api.getWebsiteHistory();
    } catch (failure) {
      modal.body.innerHTML = `<p class="text-[12.5px] text-err">${escapeHtml(failure.message)}</p>`;
      return;
    }

    const rows = [
      ["Domain", website.domain],
      ["URL", website.url],
      ["Project", website.projectPath],
      ["Document root", website.documentRoot],
      ["Project type", website.projectType],
      ["Origin", originLabel(website.origin)],
      ["Apache config", website.status.configPath],
      ["Config owner", website.status.configOwner],
      ["Hosts entry", website.status.hostsState.state],
      ["PHP-FPM", vhost ? "served through php-fpm" : "not configured"],
      ["Created", relativeTime(website.createdAtMs)],
      ["Updated", relativeTime(website.updatedAtMs)],
    ];

    const relevant = history.filter((entry) => entry.website === website.name).slice(0, 4);

    modal.body.innerHTML = `
      <dl class="grid gap-x-4 gap-y-2 sm:grid-cols-[150px_1fr]">
        ${rows
          .map(
            ([label, value]) => `
          <dt class="text-[11.5px] text-muted">${label}</dt>
          <dd class="min-w-0 break-words font-mono text-[12px] text-ink-soft">${escapeHtml(value ?? "—")}</dd>`,
          )
          .join("")}
      </dl>
      ${
        relevant.length
          ? `<div class="mt-5 border-t border-line pt-4">
              <p class="text-[11px] font-semibold tracking-wide text-muted uppercase">Recent activity</p>
              <ul class="mt-2 grid gap-1.5 text-[12px] text-ink-soft">
                ${relevant
                  .map(
                    (entry) =>
                      `<li class="flex items-center gap-2"><span class="${entry.success ? "text-ok" : "text-err"}">${entry.success ? icons.check(13) : icons.alert(13)}</span><span class="font-medium">${escapeHtml(entry.operation)}</span><span class="text-muted">${escapeHtml(entry.summary)}</span><span class="ml-auto text-muted">${relativeTime(entry.atMs)}</span></li>`,
                  )
                  .join("")}
              </ul>
            </div>`
          : ""
      }
      ${
        website.state !== "live"
          ? `<div class="mt-4 rounded-xl border border-warn/40 bg-warn/10 px-3 py-2 text-[11.5px] text-warn">${
              (website.status.warnings ?? []).map((warning) => `<p>• ${escapeHtml(warning)}</p>`).join("") ||
              escapeHtml(website.detail ?? "This website is not fully live.")
            }</div>`
          : ""
      }
    `;
  }

  function showRename(website) {
    const body = document.createElement("div");
    body.className = "grid gap-2";
    body.innerHTML = `
      <label for="rename-domain" class="text-[12px] font-medium text-ink-soft">New domain</label>
      <input id="rename-domain" class="field w-full font-mono text-[12.5px]" value="${escapeHtml(website.domain)}" spellcheck="false" autocomplete="off" />
      <p class="text-[11.5px] text-muted">The Apache VirtualHost and the hosts entry are renamed together; the duplicate entries siteForge used to leave behind are cleaned up.</p>
    `;
    openModal({
      title: "Rename domain",
      description: `${website.name} · currently ${website.domain}`,
      body,
      width: "max-w-lg",
      actions: [
        { label: "Cancel", onClick: (modal) => modal.close() },
        {
          label: "Rename",
          primary: true,
          onClick: async (modal) => {
            const value = body.querySelector("input").value.trim();
            if (!value) {
              modal.setError("Enter a domain.");
              return;
            }
            if (value === website.domain) {
              modal.close();
              return;
            }
            modal.setError("");
            modal.setBusy(true, "Renaming…");
            try {
              const report = await api.renameWebsite(website.id, value);
              modal.close();
              announceResult(report, "Domain renamed");
              await load();
            } catch (failure) {
              modal.setBusy(false);
              reportFailure(modal, failure);
            }
          },
        },
      ],
    });
  }

  async function showRemove(website) {
    const confirmed = await confirmDialog({
      title: `Remove ${website.name}?`,
      description: "The following will be removed with one authenticated step:",
      confirmLabel: "Remove website",
      details: [
        `Apache VirtualHost <code class="font-mono">${escapeHtml(website.domain)}.conf</code>`,
        `the hosts entry for <code class="font-mono">${escapeHtml(website.domain)}</code>`,
        `SELinux file contexts owned by Servora`,
        "Your project files, database and backups stay exactly where they are.",
      ],
    });
    if (!confirmed) return;

    const controller = openModal({
      title: "Removing website",
      body: `<div class="flex items-center gap-2 text-[12.5px] text-muted">${inlineSpinner(15)} Waiting for authorization…</div>`,
      width: "max-w-md",
      actions: [],
    });
    try {
      const report = await api.removeWebsite(website.id);
      controller.close();
      announceResult(report, "Website removed");
      await load();
    } catch (failure) {
      controller.close();
      toastError(failure, "Could not remove the website");
    }
  }

  function showImport() {
    openMigrationDialog({
      onImported: async () => {
        await load();
      },
    });
  }

  function announceResult(report, title) {
    if (!report?.success) {
      toast({
        tone: "warn",
        title: `${title} — rolled back`,
        message: report?.message ?? "The operation did not complete.",
        hint: report?.outcome?.rollback === "partial" ? "Some steps could not be undone; check the diagnostics page." : "",
      });
      return;
    }
    const warnings = report.warnings ?? [];
    toast({
      tone: warnings.length ? "info" : "ok",
      title,
      message: warnings.length ? warnings[0] : `${report.websiteName ?? ""} is now served by Apache.`,
    });
    for (const warning of warnings.slice(1)) {
      toast({ tone: "info", title: "Note", message: warning });
    }
  }

  function reportFailure(modal, failure) {
    if (failure instanceof ServoraError) {
      if (failure.code === "authorization_cancelled") {
        modal.setError(
          "Authorization was cancelled.",
          "Nothing was changed. Run the action again and approve the Polkit prompt.",
        );
        return;
      }
      modal.setError(failure.message, failure.hint ?? "");
      return;
    }
    modal.setError(failure?.message ?? "Unexpected error");
  }

  // -------------------------------------------------------------- listeners

  search.addEventListener("input", () => {
    query = search.value;
    render();
  });

  element.querySelectorAll("[data-filter]").forEach((button) => {
    button.addEventListener("click", () => {
      filter = button.dataset.filter;
      element
        .querySelectorAll("[data-filter]")
        .forEach((other) => other.setAttribute("aria-pressed", String(other === button)));
      render();
    });
  });

  element.querySelectorAll("[data-sort]").forEach((button) => {
    button.addEventListener("click", () => {
      sort = button.dataset.sort;
      element
        .querySelectorAll("[data-sort]")
        .forEach((other) => other.setAttribute("aria-pressed", String(other === button)));
      render();
    });
  });

  refreshButton.addEventListener("click", () => {
    refreshButton.disabled = true;
    refreshButton.classList.add("animate-spin");
    load().finally(() => {
      refreshButton.disabled = false;
      refreshButton.classList.remove("animate-spin");
    });
  });

  element.querySelector('[data-role="create"]').addEventListener("click", showCreate);
  element.querySelector('[data-role="import"]').addEventListener("click", showImport);

  let disposed = false;
  let unsubscribe = null;

  return {
    element,
    mount() {
      load();
      // Refresh when the window regains focus, so external changes show up.
      const onFocus = () => {
        if (!disposed) load();
      };
      window.addEventListener("focus", onFocus);
      unsubscribe = () => window.removeEventListener("focus", onFocus);
    },
    unmount() {
      disposed = true;
      unsubscribe?.();
    },
  };
}

function shortPath(path) {
  if (!path) return "—";
  const home = "/home/";
  if (path.startsWith(home)) {
    const parts = path.split("/");
    return `~/${parts.slice(3).join("/")}`;
  }
  return path;
}

async function openUrl(url) {
  try {
    const { openUrl: open } = await import("@tauri-apps/plugin-opener");
    await open(url);
  } catch {
    window.open(url, "_blank", "noopener");
  }
}
