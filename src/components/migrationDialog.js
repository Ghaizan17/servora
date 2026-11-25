/**
 * siteForge migration dialog.
 *
 * Servora never claims a site silently. The dialog shows exactly what was found
 * on disk (VirtualHost files, the hosts file and the project directories), marks
 * the legacy or hand written ones, and imports only what the user ticks.
 * Nothing is deleted, and Servora rewrites the configuration so it no longer
 * depends on siteForge.
 */

import { api } from "../lib/ipc.js";
import { icons } from "../lib/icons.js";
import { escapeHtml } from "./modal.js";
import { openModal } from "./modal.js";
import { toast, toastError } from "./toast.js";
import { inlineSpinner, placeholder } from "./ui.js";

export function openMigrationDialog({ onImported }) {
  const body = document.createElement("div");
  body.className = "grid gap-3";

  const modal = openModal({
    title: "Import existing websites",
    description:
      "Servora scans /etc/httpd/conf.d and /etc/hosts, then takes over the VirtualHosts you select.",
    body,
    width: "max-w-3xl",
    actions: [
      { label: "Cancel", onClick: (controller) => controller.close() },
      {
        label: "Import selected",
        primary: true,
        disabled: true,
        onClick: async (controller) => {
          const selected = [...body.querySelectorAll("input[data-domain]:checked")].map(
            (input) => input.dataset.domain,
          );
          if (selected.length === 0) return;
          controller.setError("");
          controller.setBusy(true, "Importing…");
          try {
            const report = await api.importWebsites(selected);
            controller.setBusy(false);
            controller.close();
            const imported = report.imported.length;
            const failed = report.failed.length;
            if (imported > 0) {
              toast({
                tone: failed ? "info" : "ok",
                title: `${imported} website${imported === 1 ? "" : "s"} imported`,
                message: failed
                  ? `${failed} could not be imported; nothing was left half written.`
                  : "The configurations are now managed by Servora.",
              });
            }
            for (const skip of report.skipped) toast({ tone: "warn", title: "Skipped", message: skip });
            for (const failure of report.failed) {
              toast({
                tone: "err",
                title: `Import failed: ${failure.websiteName ?? "website"}`,
                message: failure.message ?? "The operation was rolled back.",
              });
            }
            onImported?.();
          } catch (error) {
            controller.setBusy(false);
            toastError(error, "Import failed");
          }
        },
      },
    ],
  });

  const importButton = modal.buttons[modal.buttons.length - 1];

  body.innerHTML = `<div class="flex items-center gap-2 py-6 text-[12.5px] text-muted">${inlineSpinner(15)} Scanning Apache and the hosts file…</div>`;

  api
    .previewMigration()
    .then((scan) => renderScan(body, scan, importButton))
    .catch((error) => {
      body.innerHTML = "";
      body.append(
        placeholder({
          icon: icons.alert(20),
          title: "The scan failed",
          message: error.message,
          tone: "danger",
        }),
      );
    });
}

function renderScan(body, scan, importButton) {
  const candidates = scan.candidates ?? [];
  body.innerHTML = "";

  if (candidates.length === 0) {
    body.append(
      placeholder({
        icon: icons.check(20),
        title: "Nothing to import",
        message:
          "No unmanaged VirtualHost files were found in /etc/httpd/conf.d. New websites can be created directly.",
        tone: "muted",
      }),
    );
    return;
  }

  const summary = document.createElement("div");
  summary.className = "flex flex-wrap items-center gap-x-4 gap-y-1 text-[11.5px] text-muted";
  const importable = candidates.filter((candidate) => candidate.importable);
  summary.innerHTML = `
    <span>${candidates.length} configuration${candidates.length === 1 ? "" : "s"} found</span>
    <span class="${importable.length ? "text-ok" : ""}">${importable.length} importable</span>
    ${scan.siteforgeInstalled ? '<span class="badge">siteForge installed</span>' : ""}
    <span class="ml-auto font-mono">${escapeHtml(scan.confDir)}</span>
  `;

  const list = document.createElement("div");
  list.className = "mt-1 flex max-h-[46vh] flex-col gap-2 overflow-y-auto pr-1";

  for (const candidate of candidates) {
    const card = document.createElement("label");
    card.className = `flex gap-3 rounded-xl border px-3.5 py-3 transition ${
      candidate.importable
        ? "border-line bg-panel-soft/60 hover:border-line-strong"
        : "border-line/60 bg-panel-soft/30 opacity-70"
    }`;
    card.innerHTML = `
      <input type="checkbox" data-domain="${escapeHtml(candidate.domain)}" class="mt-1 h-4 w-4 flex-none accent-[var(--color-accent)]"
        ${candidate.importable ? "" : "disabled"} />
      <div class="min-w-0 flex-1">
        <div class="flex flex-wrap items-center gap-2">
          <span class="font-mono text-[12.5px] font-medium text-ink">${escapeHtml(candidate.domain)}</span>
          <span class="badge">${escapeHtml(candidate.owner)}</span>
          ${candidate.alreadyImported ? '<span class="badge state-active"><span class="dot"></span>already imported</span>' : ""}
        </div>
        <p class="mt-1 truncate font-mono text-[11px] text-muted" title="${escapeHtml(candidate.documentRoot)}">${escapeHtml(candidate.projectPath)}</p>
        ${candidate.warnings?.length ? `<p class="mt-1 text-[11px] leading-relaxed text-warn">${candidate.warnings.map((warning) => escapeHtml(warning)).join(" · ")}</p>` : ""}
      </div>
    `;
    list.append(card);
  }

  body.append(summary, list);

  const selectAll = document.createElement("div");
  selectAll.className = "mt-1 flex items-center gap-3 text-[11.5px] text-muted";
  selectAll.innerHTML = `
    <button type="button" data-role="select-all" class="btn btn-ghost">Select all importable</button>
    <span data-role="count"></span>
  `;
  body.append(selectAll);

  const count = selectAll.querySelector('[data-role="count"]');
  const checkboxes = [...body.querySelectorAll("input[data-domain]")];

  function sync() {
    const chosen = checkboxes.filter((box) => box.checked).length;
    count.textContent = chosen ? `${chosen} selected` : "Nothing selected";
    importButton.disabled = chosen === 0 || modalBusy;
  }

  let modalBusy = false;
  checkboxes.forEach((box) => box.addEventListener("change", sync));

  selectAll.querySelector('[data-role="select-all"]').addEventListener("click", () => {
    const boxes = checkboxes.filter((box) => !box.disabled);
    const allChecked = boxes.every((box) => box.checked);
    boxes.forEach((box) => {
      box.checked = !allChecked;
    });
    sync();
  });

  const recommended = candidates.filter((candidate) => candidate.importable && !candidate.alreadyImported);
  if (recommended.length) {
    recommended.forEach((candidate) => {
      const box = checkboxes.find((input) => input.dataset.domain === candidate.domain);
      if (box) box.checked = true;
    });
  }
  sync();
}
