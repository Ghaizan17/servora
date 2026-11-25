/**
 * Diagnostics — health checks plus the first-run environment matrix.
 *
 * Nothing on this page changes the system except the explicit "Reload Apache"
 * action, which goes through the helper and Polkit like every other write.
 */

import { api, ServoraError } from "../lib/ipc.js";
import { icons } from "../lib/icons.js";
import { escapeHtml } from "../lib/format.js";
import { inlineSpinner, placeholder } from "../components/ui.js";
import { toast, toastError } from "../components/toast.js";

const STATUS_STYLE = {
  passed: { className: "state-active", icon: icons.check, label: "Passed" },
  warning: { className: "state-pending", icon: icons.alert, label: "Warning" },
  failed: { className: "state-failed", icon: icons.alert, label: "Failed" },
  notApplicable: { className: "state-inactive", icon: icons.info, label: "Not applicable" },
};

export function Diagnostics() {
  const element = document.createElement("div");
  element.className = "flex min-h-0 flex-1 flex-col gap-4";

  element.innerHTML = `
    <div class="flex flex-wrap items-center gap-2">
      <div class="segmented" role="group" aria-label="Diagnostics view">
        <button data-view="checks" aria-pressed="true">Health checks</button>
        <button data-view="environment" aria-pressed="false">Environment</button>
      </div>
      <div class="ml-auto flex flex-wrap items-center gap-2">
        <button data-role="validate" class="btn">${icons.shield(14)} Validate Apache</button>
        <button data-role="reload" class="btn">${icons.restart(14)} Reload Apache</button>
        <button data-role="refresh" class="btn btn-primary">${icons.refresh(14)} Re-run checks</button>
      </div>
    </div>
    <div data-role="host" class="flex flex-wrap items-center gap-x-4 gap-y-1 text-[11.5px] text-muted"></div>
    <div data-role="body" class="scroll-area min-h-0 flex-1"></div>
  `;

  const body = element.querySelector('[data-role="body"]');
  const host = element.querySelector('[data-role="host"]');
  const refreshButton = element.querySelector('[data-role="refresh"]');
  const validateButton = element.querySelector('[data-role="validate"]');
  const reloadButton = element.querySelector('[data-role="reload"]');

  let view = "checks";
  let report = null;
  let environment = null;
  let busy = false;

  function statusBadge(status) {
    const style = STATUS_STYLE[status] ?? STATUS_STYLE.notApplicable;
    return `<span class="badge ${style.className}"><span class="dot"></span>${style.label}</span>`;
  }

  function renderHost(info) {
    if (!info) return;
    host.innerHTML = `
      ${info.fedoraVersion ? `<span>Fedora ${escapeHtml(info.fedoraVersion)}</span>` : "<span>Fedora (version unknown)</span>"}
      <span class="font-mono">${escapeHtml(info.architecture)}</span>
      ${info.kernel ? `<span class="font-mono">${escapeHtml(info.kernel)}</span>` : ""}
      <span>SELinux: <span class="${info.selinux === "Enforcing" ? "text-ok" : "text-warn"}">${escapeHtml(info.selinux)}</span></span>
      <span class="truncate">${escapeHtml(info.user)}@${escapeHtml(info.home)}</span>
    `;
  }

  function renderChecks() {
    body.innerHTML = "";
    if (!report) {
      body.innerHTML = `<div class="flex items-center justify-center gap-2 py-16 text-[12.5px] text-muted">${inlineSpinner(16)} Running checks…</div>`;
      return;
    }

    const { summary, checks } = report;
    const banner = document.createElement("div");
    const tone =
      summary.failed > 0 ? "err" : summary.warnings > 0 ? "warn" : "ok";
    const toneClasses = {
      err: "border-err/40 bg-err/10 text-err",
      warn: "border-warn/40 bg-warn/10 text-warn",
      ok: "border-ok/40 bg-ok/10 text-ok",
    };
    banner.className = `mb-3 flex flex-wrap items-center gap-3 rounded-xl border px-4 py-3 text-[12.5px] ${toneClasses[tone]}`;
    banner.innerHTML = `
      <span class="font-medium">${
        summary.failed > 0
          ? `${summary.failed} check${summary.failed === 1 ? "" : "s"} failed`
          : summary.warnings > 0
            ? `${summary.warnings} warning${summary.warnings === 1 ? "" : "s"}`
            : "Everything looks healthy"
      }</span>
      <span class="text-muted">${summary.passed} passed · ${summary.warnings} warning${summary.warnings === 1 ? "" : "s"} · ${summary.failed} failed · ${summary.total} total</span>
      <button data-role="copy" class="btn btn-ghost ml-auto">${icons.copy(13)} Copy report</button>
    `;
    body.append(banner);
    banner.querySelector('[data-role="copy"]').addEventListener("click", copyReport);

    const list = document.createElement("div");
    list.className = "grid gap-2";
    for (const check of checks) {
      const row = document.createElement("article");
      row.className = "card px-4 py-3";
      row.innerHTML = `
        <div class="flex flex-wrap items-center gap-2">
          <h3 class="text-[13px] font-semibold text-ink">${escapeHtml(check.label)}</h3>
          ${statusBadge(check.status)}
        </div>
        <p class="mt-1 text-[12px] leading-relaxed text-ink-soft">${escapeHtml(check.detail)}</p>
        ${
          check.remediation
            ? `<p class="mt-1.5 rounded-lg border border-line bg-panel-soft px-2.5 py-1.5 font-mono text-[11.5px] text-muted">${escapeHtml(check.remediation)}</p>`
            : ""
        }
      `;
      list.append(row);
    }
    body.append(list);
  }

  function renderEnvironment() {
    body.innerHTML = "";
    if (!environment) {
      body.innerHTML = `<div class="flex items-center justify-center gap-2 py-16 text-[12.5px] text-muted">${inlineSpinner(16)} Checking the environment…</div>`;
      return;
    }

    const intro = document.createElement("div");
    intro.className = `mb-3 rounded-xl border px-4 py-3 text-[12.5px] ${
      environment.ready
        ? "border-ok/40 bg-ok/10 text-ok"
        : "border-warn/40 bg-warn/10 text-warn"
    }`;
    intro.innerHTML = environment.ready
      ? "Every dependency Servora needs is installed."
      : "Some dependencies are missing. Install them with the commands below, then re-run the checks.";
    body.append(intro);

    const grid = document.createElement("div");
    grid.className = "grid gap-2 min-[900px]:grid-cols-2";
    for (const component of environment.components) {
      const style = STATUS_STYLE[component.status] ?? STATUS_STYLE.notApplicable;
      const card = document.createElement("article");
      card.className = "card flex flex-col gap-1 px-4 py-3";
      card.innerHTML = `
        <div class="flex flex-wrap items-center gap-2">
          <span class="text-${component.status === "passed" ? "ok" : component.status === "failed" ? "err" : "warn"}">${style.icon(14)}</span>
          <h3 class="text-[13px] font-semibold text-ink">${escapeHtml(component.label)}</h3>
        </div>
        <p class="text-[11.5px] leading-relaxed text-muted">${escapeHtml(component.detail)}</p>
        ${
          component.package
            ? `<code class="mt-1 block truncate rounded-lg border border-line bg-panel-soft px-2.5 py-1.5 font-mono text-[11px] text-ink-soft" title="${escapeHtml(component.package)}">sudo dnf install ${escapeHtml(component.package)}</code>`
            : ""
        }
      `;
      grid.append(card);
    }
    body.append(grid);
  }

  function render() {
    renderHost((view === "checks" ? report?.host : environment?.host) ?? null);
    if (view === "checks") renderChecks();
    else renderEnvironment();
  }

  async function loadChecks() {
    report = null;
    render();
    try {
      report = await api.getDiagnostics();
    } catch (error) {
      toastError(error, "Diagnostics failed");
      body.innerHTML = "";
      body.append(
        placeholder({
          icon: icons.alert(20),
          title: "The checks could not run",
          message: error.message,
          tone: "danger",
          action: { label: "Retry", onClick: () => loadChecks() },
        }),
      );
      return;
    }
    render();
  }

  async function loadEnvironment() {
    environment = null;
    render();
    try {
      environment = await api.getEnvironment();
    } catch (error) {
      toastError(error, "Environment check failed");
    }
    render();
  }

  function copyReport() {
    const lines = [
      `Servora diagnostics — ${new Date(report.generatedAtMs).toLocaleString()}`,
      `Host: Fedora ${report.host.fedoraVersion ?? "?"} ${report.host.architecture}, kernel ${report.host.kernel ?? "?"}`,
      `SELinux: ${report.host.selinux}`,
      "",
      ...report.checks.map(
        (check) =>
          `[${check.status}] ${check.label}: ${check.detail}${check.remediation ? ` → ${check.remediation}` : ""}`,
      ),
    ];
    navigator.clipboard
      .writeText(lines.join("\n"))
      .then(() => toast({ tone: "ok", title: "Report copied", message: "Paste it into a bug report." }))
      .catch(() => toast({ tone: "warn", title: "Clipboard unavailable", message: "Could not copy the report." }));
  }

  async function runApacheCheck() {
    validateButton.disabled = true;
    try {
      const result = await api.validateApache();
      toast({
        tone: result.valid ? "ok" : "err",
        title: result.valid ? "Apache configuration is valid" : "Apache rejected the configuration",
        message: String(result.output ?? "").trim().split("\n").slice(-2).join(" ") || "No output.",
      });
    } catch (error) {
      toastError(error, "Config test failed");
    } finally {
      validateButton.disabled = false;
    }
  }

  async function reloadApache() {
    reloadButton.disabled = true;
    const pending = toast({
      tone: "info",
      title: "Reloading Apache",
      message: "Approve the Polkit prompt if one appears.",
      timeout: 8000,
    });
    try {
      const outcome = await api.reloadApache();
      if (outcome.success) {
        toast({ tone: "ok", title: "Apache reloaded", message: "The running configuration was refreshed." });
      } else {
        toast({
          tone: "err",
          title: "Reload failed",
          message: outcome.warnings?.[0] ?? "Check the Apache error log.",
        });
      }
    } catch (error) {
      if (error instanceof ServoraError && error.code === "authorization_cancelled") {
        toast({
          tone: "info",
          title: "Reload cancelled",
          message: "Nothing changed.",
        });
      } else {
        toastError(error, "Reload failed");
      }
    } finally {
      reloadButton.disabled = false;
      void pending;
    }
  }

  element.querySelectorAll("[data-view]").forEach((button) => {
    button.addEventListener("click", () => {
      view = button.dataset.view;
      element
        .querySelectorAll("[data-view]")
        .forEach((other) => other.setAttribute("aria-pressed", String(other === button)));
      render();
      if (view === "checks" && !report) loadChecks();
      if (view === "environment" && !environment) loadEnvironment();
    });
  });

  refreshButton.addEventListener("click", async () => {
    if (busy) return;
    busy = true;
    refreshButton.disabled = true;
    try {
      if (view === "checks") await loadChecks();
      else await loadEnvironment();
    } finally {
      busy = false;
      refreshButton.disabled = false;
    }
  });

  validateButton.addEventListener("click", runApacheCheck);
  reloadButton.addEventListener("click", reloadApache);

  return {
    element,
    mount() {
      loadChecks();
    },
  };
}
