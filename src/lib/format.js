/** Small formatting helpers shared by the pages. */

const STATE_LABELS = {
  active: "Active",
  inactive: "Inactive",
  activating: "Activating",
  deactivating: "Deactivating",
  failed: "Failed",
  unknown: "Unknown",
};

export function stateLabel(state) {
  return STATE_LABELS[state] ?? "Unknown";
}

export function stateClass(state) {
  switch (state) {
    case "active":
      return "state-active";
    case "activating":
    case "deactivating":
      return "state-pending";
    case "failed":
      return "state-failed";
    default:
      return "state-inactive";
  }
}

export function isRunning(state) {
  return state === "active";
}

export function isTransitioning(state) {
  return state === "activating" || state === "deactivating";
}

/** `14:03:22` in the user's locale. */
export function clockTime(epochMs) {
  if (!epochMs) return "—";
  return new Date(epochMs).toLocaleTimeString(undefined, {
    hour: "2-digit",
    minute: "2-digit",
    second: "2-digit",
    hour12: false,
  });
}

/** `just now`, `12s ago`, ... */
export function relativeTime(epochMs) {
  if (!epochMs) return "never";
  const seconds = Math.max(0, Math.round((Date.now() - epochMs) / 1000));
  if (seconds < 5) return "just now";
  if (seconds < 60) return `${seconds}s ago`;
  const minutes = Math.round(seconds / 60);
  if (minutes < 60) return `${minutes}m ago`;
  const hours = Math.round(minutes / 60);
  return `${hours}h ago`;
}

/** Human readable duration for operation results. */
export function duration(ms) {
  if (!Number.isFinite(ms)) return "";
  if (ms < 1000) return `${Math.round(ms)} ms`;
  return `${(ms / 1000).toFixed(1)} s`;
}

export function escapeHtml(value) {
  return String(value ?? "")
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/"/g, "&quot;")
    .replace(/'/g, "&#39;");
}

export const SERVICE_META = [
  { id: "httpd", short: "HTTPD", name: "Apache HTTP Server", unit: "httpd.service" },
  { id: "php-fpm", short: "PHP-FPM", name: "PHP-FPM", unit: "php-fpm.service" },
  { id: "mariadb", short: "MariaDB", name: "MariaDB", unit: "mariadb.service" },
];
