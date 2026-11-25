/**
 * Create / edit website form.
 *
 * The form never decides anything itself: paths, document roots and project
 * types are resolved by the Rust core through `detect_project_type`, and domain
 * validity comes from `validate_domain`. The UI only collects input and shows
 * what the backend reported.
 */

import { api, ServoraError } from "../lib/ipc.js";
import { icons } from "../lib/icons.js";
import { escapeHtml } from "./modal.js";

const TYPE_LABELS = {
  laravel: "Laravel",
  wordpress: "WordPress",
  php: "Generic PHP",
  unknown: "Unknown",
};

function field({ id, label, hint = "", control, prefix = "" }) {
  return `
    <div class="grid gap-1.5">
      <label for="${id}" class="text-[12px] font-medium text-ink-soft">${label}</label>
      <div class="flex items-stretch gap-2">
        ${prefix}
        ${control}
      </div>
      ${hint ? `<p data-hint="${id}" class="text-[11.5px] leading-relaxed text-muted">${hint}</p>` : ""}
    </div>
  `;
}

/**
 * @param {object} options
 * @param {"create"|"edit"} [options.mode]
 * @param {object|null} [options.initial] website record when editing
 */
export function createWebsiteForm({ mode = "create", initial = null } = {}) {
  const element = document.createElement("div");
  element.className = "grid gap-4";

  element.innerHTML = `
    ${field({
      id: "sf-path",
      label: "Project directory",
      control: `<input id="sf-path" data-role="path" class="field min-w-0 flex-1 font-mono text-[12.5px]" type="text" spellcheck="false" autocomplete="off" placeholder="/home/you/Work/my-project" value="${escapeHtml(initial?.projectPath ?? "")}" />`,
      prefix: `<button type="button" data-role="browse" class="btn shrink-0" title="Choose a folder">${icons.folder(15)}<span class="hidden sm:inline">Browse</span></button>`,
    })}

    <div data-role="detection" class="rounded-xl border border-line bg-panel-soft px-3.5 py-3 text-[12px] text-muted">
      ${icons.info(14)} <span data-role="detection-text">Pick a folder to detect the project type and document root.</span>
    </div>

    <div class="grid gap-4 min-[720px]:grid-cols-2">
      ${field({
        id: "sf-name",
        label: "Website name",
        control: `<input id="sf-name" data-role="name" class="field w-full" type="text" spellcheck="false" autocomplete="off" placeholder="my-project" value="${escapeHtml(initial?.name ?? "")}" />`,
      })}
      ${field({
        id: "sf-domain",
        label: "Domain",
        control: `<input id="sf-domain" data-role="domain" class="field w-full font-mono text-[12.5px]" type="text" spellcheck="false" autocomplete="off" placeholder="my-project.test" value="${escapeHtml(initial?.domain ?? "")}" />`,
        hint: "`.test` and `.localhost` stay on this machine. Any other domain is written to /etc/hosts too.",
      })}
    </div>

    <p data-role="domain-status" class="hidden -mt-1 text-[11.5px]"></p>

    <div class="grid gap-4 min-[720px]:grid-cols-2">
      ${field({
        id: "sf-root",
        label: "Document root",
        control: `<input id="sf-root" data-role="root" class="field w-full font-mono text-[12.5px]" type="text" spellcheck="false" autocomplete="off" placeholder="detected automatically" value="${escapeHtml(initial?.documentRoot ?? "")}" />`,
      })}
      ${field({
        id: "sf-type",
        label: "Project type",
        control: `<select id="sf-type" data-role="type" class="field w-full">
            ${Object.entries(TYPE_LABELS)
              .map(
                ([value, label]) =>
                  `<option value="${value}"${(initial?.projectType ?? "unknown") === value ? " selected" : ""}>${label}</option>`,
              )
              .join("")}
          </select>`,
      })}
    </div>
    <p data-role="warnings" class="hidden rounded-xl border border-warn/40 bg-warn/10 px-3.5 py-2.5 text-[12px] leading-relaxed text-warn"></p>
  `;

  const path = element.querySelector('[data-role="path"]');
  const name = element.querySelector('[data-role="name"]');
  const domain = element.querySelector('[data-role="domain"]');
  const root = element.querySelector('[data-role="root"]');
  const type = element.querySelector('[data-role="type"]');
  const detection = element.querySelector('[data-role="detection"]');
  const detectionText = element.querySelector('[data-role="detection-text"]');
  const warnings = element.querySelector('[data-role="warnings"]');
  const domainStatus = element.querySelector('[data-role="domain-status"]');

  let nameTouched = Boolean(initial?.name);
  let domainTouched = Boolean(initial?.domain);
  let rootTouched = Boolean(initial?.documentRoot);
  let inspecting = false;

  name.addEventListener("input", () => {
    nameTouched = name.value.trim().length > 0;
    if (!domainTouched) domain.value = name.value.trim() ? `${slug(name.value)}.test` : "";
    refreshDomain();
  });
  domain.addEventListener("input", () => {
    domainTouched = domain.value.trim().length > 0;
    refreshDomain();
  });
  root.addEventListener("input", () => {
    rootTouched = root.value.trim().length > 0;
  });

  let domainTimer = null;
  function refreshDomain() {
    clearTimeout(domainTimer);
    const value = domain.value.trim();
    if (!value) {
      domainStatus.classList.add("hidden");
      return;
    }
    domainTimer = setTimeout(async () => {
      try {
        const result = await api.validateDomain(value);
        if (result.valid) {
          domainStatus.className = "text-[11.5px] text-ok";
          domainStatus.innerHTML = `${icons.check(13)} ${escapeHtml(result.domain)} resolves locally — ${escapeHtml(result.url)}${result.takenByServora ? " · already used by another Servora website" : ""}`;
        } else {
          domainStatus.className = "text-[11.5px] text-err";
          domainStatus.innerHTML = `${icons.alert(13)} ${escapeHtml(result.message ?? "Invalid domain")}`;
        }
        domainStatus.classList.remove("hidden");
      } catch {
        domainStatus.classList.add("hidden");
      }
    }, 220);
  }

  async function inspect() {
    const value = path.value.trim();
    if (!value || inspecting) return;
    inspecting = true;
    detection.className =
      "rounded-xl border border-line bg-panel-soft px-3.5 py-3 text-[12px] text-muted";
    detectionText.textContent = "Inspecting the project…";

    try {
      const inspection = await api.detectProject(value);
      const label = TYPE_LABELS[inspection.projectType] ?? "Unknown";
      const reasons = inspection.detectedBy?.length ? inspection.detectedBy.join(", ") : "directory layout";
      detection.className =
        "rounded-xl border border-line bg-panel-soft px-3.5 py-3 text-[12px] text-ink-soft";
      detection.innerHTML = `
        ${icons.check(14)}
        <span><strong class="text-ink">${escapeHtml(label)}</strong> detected via ${escapeHtml(reasons)}${
          inspection.documentRoot
            ? ` · document root <code class="font-mono text-[11.5px] text-ink-soft">${escapeHtml(inspection.documentRoot.replace(`${value.replace(/\/$/, "")}/`, ""))}</code>`
            : ""
        }</span>
      `;

      if (!rootTouched && inspection.documentRoot) root.value = inspection.documentRoot;
      if (inspection.projectType === "unknown") type.value = "unknown";
      else type.value = inspection.projectType;

      if (!nameTouched) {
        const base = value.replace(/\/+$/, "").split("/").pop() ?? "";
        if (base) {
          name.value = base;
          if (!domainTouched) domain.value = `${slug(base)}.test`;
          refreshDomain();
        }
      }

      const messages = [...(inspection.warnings ?? [])];
      if (messages.length) {
        warnings.innerHTML = messages.map((message) => `• ${escapeHtml(message)}`).join("<br/>");
        warnings.classList.remove("hidden");
      } else {
        warnings.classList.add("hidden");
      }
    } catch (error) {
      const hint = error instanceof ServoraError ? error.hint : "";
      detection.className =
        "rounded-xl border border-err/40 bg-err/10 px-3.5 py-3 text-[12px] text-err";
      detection.innerHTML = `${icons.alert(14)} <span>${escapeHtml(error.message)}${hint ? ` <span class="text-err/80">${escapeHtml(hint)}</span>` : ""}</span>`;
      warnings.classList.add("hidden");
    } finally {
      inspecting = false;
    }
  }

  let pathTimer = null;
  path.addEventListener("input", () => {
    clearTimeout(pathTimer);
    pathTimer = setTimeout(inspect, 400);
  });
  path.addEventListener("blur", inspect);

  element.querySelector('[data-role="browse"]').addEventListener("click", async () => {
    try {
      const { open } = await import("@tauri-apps/plugin-dialog");
      const selected = await open({
        directory: true,
        multiple: false,
        title: "Select the project folder",
      });
      if (typeof selected === "string" && selected) {
        path.value = selected;
        await inspect();
      }
    } catch {
      detection.className =
        "rounded-xl border border-warn/40 bg-warn/10 px-3.5 py-3 text-[12px] text-warn";
      detectionText.textContent =
        "The native folder picker is unavailable here — paste the absolute path instead.";
    }
  });

  if (initial?.projectPath) {
    requestAnimationFrame(inspect);
  } else {
    requestAnimationFrame(() => path.focus());
  }
  if (initial?.domain) requestAnimationFrame(refreshDomain);

  return {
    element,
    mode,
    focus: () => path.focus(),
    /** @returns {{name:string, domain:string, projectPath:string, documentRoot:string|null, projectType:string|null}} */
    payload() {
      return {
        name: name.value.trim(),
        domain: domain.value.trim(),
        projectPath: path.value.trim(),
        documentRoot: root.value.trim() || null,
        projectType: type.value || null,
      };
    },
    validate() {
      const value = this.payload();
      if (!value.projectPath) return "Choose the project directory first.";
      if (!value.name) return "Give the website a name.";
      if (!value.domain) return "A domain is required.";
      return null;
    },
  };
}

/** Mirror of the Rust slug rule, used only to pre-fill the form. */
function slug(value) {
  return value
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 63) || "website";
}
