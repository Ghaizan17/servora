/**
 * IPC layer.
 *
 * Every call into Rust goes through `invoke`, and every failure is normalised
 * into a `ServoraError` that carries the stable `code` and the optional `hint`
 * produced by the backend. The UI never inspects raw error strings.
 */
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";

export class ServoraError extends Error {
  constructor(payload) {
    const message =
      typeof payload === "string"
        ? payload
        : payload?.message || "Unexpected error";
    super(message);
    this.name = "ServoraError";
    this.code = payload?.code || "unknown";
    this.hint = payload?.hint || null;
  }
}

export function isTauri() {
  return typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
}

async function call(command, args) {
  if (!isTauri()) {
    throw new ServoraError({
      code: "not_tauri",
      message:
        "Servora must run inside its desktop shell. Start it with `npm run app:dev`.",
    });
  }
  try {
    return await invoke(command, args ?? {});
  } catch (error) {
    throw error instanceof ServoraError ? error : new ServoraError(error);
  }
}

export const api = {
  getServicesStatus: () => call("get_services_status"),
  getServiceStatus: (service) => call("get_service_status", { service }),
  startService: (service) => call("start_service", { service }),
  stopService: (service) => call("stop_service", { service }),
  restartService: (service) => call("restart_service", { service }),
  startAll: () => call("start_all_services"),
  stopAll: () => call("stop_all_services"),
  restartAll: () => call("restart_all_services"),
  getServiceLogs: (request) => call("get_service_logs", { request }),
  getLogsContext: () => call("get_logs_context"),
  getSettings: () => call("get_app_settings"),
  saveSettings: (settings) => call("save_app_settings", { settings }),
  reloadSettings: () => call("reload_app_settings"),
  getAppInfo: () => call("get_app_info"),
  getAuthorizationInfo: () => call("get_authorization_info"),
  getSettingsLimits: () => call("get_settings_limits"),
};

/** Batch progress events emitted by the Rust backend. */
export function onBatchProgress(handler) {
  if (!isTauri()) return () => {};
  const unlisten = listen("servora://batch-progress", (event) =>
    handler(event.payload),
  );
  return () => {
    unlisten.then((dispose) => dispose()).catch(() => {});
  };
}

/** Fired when the tray changes something, so the dashboard can refresh. */
export function onTrayRefresh(handler) {
  if (!isTauri()) return () => {};
  const unlisten = listen("servora://tray-refresh", () => handler());
  return () => {
    unlisten.then((dispose) => dispose()).catch(() => {});
  };
}
