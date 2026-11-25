/**
 * Domains — the Domain Manager.
 *
 * Shows the domain-to-website mapping Servora manages in /etc/hosts and warns
 * about entries that belong to another tool or appear twice (a known siteForge
 * rename bug).
 */

import { api } from "../lib/ipc.js";
import { icons } from "../lib/icons.js";
import { escapeHtml } from "../lib/format.js";
import { inlineSpinner, placeholder } from "../components/ui.js";
import { toastError } from "../components/toast.js";

export function Domains() {
  const element = document.createElement("div");
  element.className = "flex min-h-0 flex-1 flex-col gap-4";

  element.innerHTML = `
    <div class="flex flex-wrap items-center gap-2">
      <div class="min-w-0 flex-1">
        <p data-role="hosts-path" class="truncate font-mono text-[11.5px] text-muted"></p>
      </div>
      <button data-role="refresh" class="btn">${icons.refresh(14)} Reload</button>
    </div>
    <div data-role="body" class="scroll-area min-h-0 flex-1"></div>
  `;

  const body = element.querySelector('[data-role="body"]');
  const hostsPath = element.querySelector('[data-role="hosts-path"]');
  const refresh = element.querySelector('[data-role="refresh"]');

  let reload = async () => {};

  function mount() {
    reload = async () => {
      body.innerHTML = `<div class="flex items-center justify-center gap-2 py-16 text-[12.5px] text-muted">${inlineSpinner(16)} Reading the hosts file…</div>`;
      try {
        const payload = await api.getManagedDomains();
        hostsPath.textContent = payload.hostsPath ?? "";
        render(payload);
      } catch (error) {
        body.innerHTML = "";
        body.append(
          placeholder({
            icon: icons.alert(20),
            title: "The hosts file could not be read",
            message: error.message,
            tone: "danger",
            action: { label: "Try again", onClick: () => reload() },
          }),
        );
      }
    };
    reload();
  }

  function render(payload) {
    body.innerHTML = "";
    const domains = payload.domains ?? [];
    const websites = payload.servoraWebsites ?? [];

    if (domains.length === 0) {
      body.append(
        placeholder({
          icon: icons.globe(22),
          title: "No managed domains yet",
          message:
            "Domains appear here as soon as a website is created. Servora keeps them inside one clearly marked block in /etc/hosts.",
          tone: "muted",
        }),
      );
    } else {
      const table = document.createElement("div");
      table.className = "card overflow-hidden";
      table.innerHTML = `
        <table class="w-full border-collapse text-left text-[12.5px]">
          <thead>
            <tr class="border-b border-line text-[11px] tracking-wide text-muted uppercase">
              <th class="px-4 py-2.5 font-semibold">Domain</th>
              <th class="px-4 py-2.5 font-semibold">Website</th>
              <th class="px-4 py-2.5 font-semibold">Address</th>
              <th class="px-4 py-2.5 font-semibold">Line</th>
            </tr>
          </thead>
          <tbody class="divide-y divide-line/70">
            ${domains
              .map(
                (entry) => `
              <tr class="hover:bg-panel-soft/60">
                <td class="px-4 py-2.5 font-mono text-ink">${escapeHtml(entry.domain)}</td>
                <td class="px-4 py-2.5 text-ink-soft">${escapeHtml(entry.website ?? "—")}</td>
                <td class="px-4 py-2.5 font-mono text-muted">${escapeHtml(entry.ip)}</td>
                <td class="px-4 py-2.5 font-mono text-muted">${escapeHtml(entry.line)}</td>
              </tr>`,
              )
              .join("")}
          </tbody>
        </table>
      `;
      body.append(table);
    }

    // Websites whose domain is not (or wrongly) in the hosts file.
    const missing = websites.filter(
      (site) => !domains.some((entry) => entry.domain === site.domain),
    );
    if (missing.length) {
      const warning = document.createElement("div");
      warning.className = "mt-4 rounded-xl border border-warn/40 bg-warn/10 px-4 py-3 text-[12px] text-warn";
      warning.innerHTML = `
        <p class="font-medium">These websites have no entry inside the Servora block</p>
        <ul class="mt-1.5 grid gap-1 font-mono text-[11.5px]">
          ${missing.map((site) => `<li>${escapeHtml(site.domain)} → ${escapeHtml(site.website)}</li>`).join("")}
        </ul>
        <p class="mt-2 text-[11.5px] text-warn/80">Edit the website and save it again to rewrite the hosts entry.</p>
      `;
      body.append(warning);
    }

    const note = document.createElement("p");
    note.className = "mt-4 text-[11.5px] leading-relaxed text-muted";
    note.textContent =
      "Only lines inside the “# BEGIN SERVORA MANAGED DOMAINS” block are rewritten. Entries added by other tools, including old siteForge lines, are left untouched and reported as unmanaged.";
    body.append(note);
  }

  refresh.addEventListener("click", () => reload());

  return { element, mount };
}
