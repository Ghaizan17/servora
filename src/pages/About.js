/**
 * About — what Servora is, how it stays unprivileged, and where it keeps data.
 */

import { api } from "../lib/ipc.js";
import { icons } from "../lib/icons.js";
import { escapeHtml } from "../lib/format.js";

export function About() {
  const element = document.createElement("div");
  element.className = "scroll-area flex min-h-0 flex-1 flex-col gap-4";

  element.innerHTML = `
    <section class="card flex flex-wrap items-center gap-4 px-5 py-4">
      <img src="/favicon.png" alt="" class="h-14 w-14 rounded-2xl border border-line object-cover" />
      <div class="min-w-0">
        <h2 class="text-[15px] font-semibold text-ink">Servora</h2>
        <p class="text-[12.5px] text-muted">Local development environment manager for Fedora.</p>
        <p data-role="version" class="mt-1 font-mono text-[11.5px] text-muted"></p>
      </div>
    </section>

    <section class="card px-5 py-4">
      <h3 class="text-[13px] font-semibold tracking-wide text-ink uppercase">How Servora works</h3>
      <ul class="mt-2 grid gap-2 text-[12.5px] leading-relaxed text-ink-soft">
        <li class="flex gap-2">${icons.shield(15)}<span>Service control uses systemd over D-Bus; Polkit asks you to authorize each change. No password is stored and no setuid helper exists.</span></li>
        <li class="flex gap-2">${icons.folder(15)}<span>Website changes (VirtualHosts, hosts entries, ACLs, SELinux contexts) run in a small helper through <code class="font-mono text-[11.5px]">pkexec</code>, only after Polkit approves them.</span></li>
        <li class="flex gap-2">${icons.alert(15)}<span>Every multi-step change is transactional: if one step fails the others are rolled back and the configuration is restored from a backup.</span></li>
        <li class="flex gap-2">${icons.info(15)}<span>Servora never disables SELinux and never writes outside <code class="font-mono text-[11.5px]">/etc/httpd/conf.d</code> and <code class="font-mono text-[11.5px]">/etc/hosts</code>.</span></li>
      </ul>
    </section>

    <section class="card px-5 py-4">
      <div class="flex items-center justify-between gap-3">
        <h3 class="text-[13px] font-semibold tracking-wide text-ink uppercase">Where data lives</h3>
        <span data-role="helper" class="badge"></span>
      </div>
      <dl data-role="paths" class="mt-3 grid gap-x-4 gap-y-2 text-[12px] sm:grid-cols-[150px_1fr]"></dl>
    </section>

    <section class="card px-5 py-4">
      <h3 class="text-[13px] font-semibold tracking-wide text-ink uppercase">Managed units</h3>
      <ul class="mt-2 grid gap-1 font-mono text-[12px] text-ink-soft">
        <li>httpd.service</li>
        <li>php-fpm.service</li>
        <li>mariadb.service</li>
      </ul>
      <p class="mt-2 text-[11.5px] text-muted">Servora refuses to control any other unit, so a compromised UI cannot reach the rest of the system.</p>
    </section>
  `;

  async function load() {
    const versionNode = element.querySelector('[data-role="version"]');
    const pathsNode = element.querySelector('[data-role="paths"]');
    const helperNode = element.querySelector('[data-role="helper"]');

    try {
      const info = await api.getAppInfo();
      versionNode.textContent = `Version ${info.version} · Tauri ${info.tauriVersion ?? ""}`.trim();
    } catch {
      versionNode.textContent = "Version unavailable";
    }

    try {
      const paths = await api.getSitePaths();
      const rows = [
        ["Configuration", paths.configDir],
        ["Website metadata", paths.dataDir],
        ["Backups", paths.backupsDir],
        ["Apache configs", paths.apacheConfDir],
        ["Hosts file", paths.hostsFile],
        ["Privileged helper", paths.helperPath],
      ];
      pathsNode.innerHTML = rows
        .map(
          ([label, value]) =>
            `<dt class="text-muted">${escapeHtml(label)}</dt><dd class="min-w-0 break-all font-mono text-[11.5px] text-ink-soft">${escapeHtml(value)}</dd>`,
        )
        .join("");
      helperNode.className = `badge ${paths.helperInstalled ? "state-active" : "state-failed"}`;
      helperNode.innerHTML = `<span class="dot"></span>${paths.helperInstalled ? "Helper installed" : "Helper missing"}`;
    } catch (error) {
      pathsNode.innerHTML = `<dd class="text-[12px] text-err">${escapeHtml(error.message)}</dd>`;
    }
  }

  return { element, mount: load };
}
