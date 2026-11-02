import { invoke } from "@tauri-apps/api/core";

export async function call(command, payload = {}) {
  try {
    return await invoke(command, payload);
  } catch (error) {
    if (typeof error === "string") throw new Error(error);
    throw error;
  }
}

export const api = {
  bootstrap: () => call("get_bootstrap_state"),
  refresh: (scopes = ["interfaces", "routes"], force = false) =>
    call("refresh_network_state", { scopes, force }),
  updateSettings: (settings) => call("update_settings", { settings }),
  previewMetric: (input) => call("preview_metric_change", { input }),
  applyMetric: (metric, confirmationToken) =>
    call("apply_metric_change", { input: { metric, confirmationToken } }),
  previewRoute: (input) => call("preview_add_route", { input }),
  addRoute: (route, confirmationToken) =>
    call("add_route", { input: { route, confirmationToken } }),
  previewEditRoute: (originalRoute, route) =>
    call("preview_edit_route", { input: { originalRoute, route } }),
  editRoute: (originalRoute, route, confirmationToken) =>
    call("edit_route", { input: { originalRoute, route, confirmationToken } }),
  previewPersistRoute: (route) => call("preview_persist_route", { route }),
  persistRoute: (route, confirmationToken) =>
    call("persist_route", { input: { route, confirmationToken } }),
  previewRemoveRoute: (route) => call("preview_remove_route", { route }),
  removeRoute: (route, confirmationToken) =>
    call("remove_route", { input: { route, confirmationToken } }),
  previewAdapter: (input) => call("preview_adapter_control", { input }),
  applyAdapter: (adapter, confirmationToken) =>
    call("apply_adapter_control", { input: { adapter, confirmationToken } }),
  wifiProfiles: (interfaceName) => call("get_wifi_profiles", { interfaceName }),
  previewWifi: (input) => call("preview_wifi_change", { input }),
  connectWifi: (input, confirmationToken) =>
    call("connect_wifi", { input: { ...input, confirmationToken } }),
  disconnectWifi: (input, confirmationToken) =>
    call("disconnect_wifi", { input: { ...input, confirmationToken } }),
  wifiAutoStatus: () => call("get_wifi_auto_connect_status"),
  previewWifiAuto: (rule) => call("preview_wifi_auto_connect", { input: rule }),
  saveWifiAuto: (rule, confirmationToken) =>
    call("save_wifi_auto_connect", { input: { rule, confirmationToken } }),
  disableWifiAuto: (adapterGuid) =>
    call("disable_wifi_auto_connect", { adapterGuid }),
  removeWifiAuto: (adapterGuid) =>
    call("remove_wifi_auto_connect", { adapterGuid }),
  profiles: () => call("get_app_config"),
  saveProfile: (profile) => call("save_profile", { profile }),
  deleteProfile: (id) => call("delete_profile", { id }),
  profilePreview: (id) => call("restore_profile_preview", { id }),
  profileRestore: (id, confirmationToken) =>
    call("restore_profile_apply", { input: { id, confirmationToken } }),
  recentSnapshot: (name) => call("capture_recent_snapshot", { name }),
  deleteRecent: (id) => call("delete_recent_snapshot", { id }),
  recentPreview: (id) => call("restore_recent_preview", { id }),
  recentRestore: (id, confirmationToken) =>
    call("restore_recent_apply", { input: { id, confirmationToken } }),
  resetPreview: () => call("reset_preview"),
  reset: (confirmationToken) =>
    call("reset_apply", { input: { confirmationToken } }),
  defaultsPreview: (selection) => call("windows_defaults_preview", { input: selection }),
  defaults: (selection, confirmationToken) =>
    call("windows_defaults_apply", { input: { ...selection, confirmationToken } }),
  diagnostics: () => call("get_diagnostics"),
  connectivityTest: () => call("run_connectivity_test"),
  lookupRoute: (host) => call("lookup_route", { host }),
  startTrace: (host, maxHops = 30) => call("start_trace", { host, maxHops }),
  cancelTrace: () => call("cancel_trace"),
  activeConnections: () => call("get_active_connections"),
  scanWifi: (adapterGuid) => call("scan_wifi_networks", { adapterGuid }),
  monitorSnapshot: () => call("get_monitor_snapshot"),
  networkEvents: () => call("get_network_events"),
  clearNetworkEvents: () => call("clear_network_events"),
  previewQuickFix: (input) => call("preview_quick_fix", { input }),
  applyQuickFix: (fix, confirmationToken) =>
    call("apply_quick_fix", { input: { fix, confirmationToken } }),
  startupStatus: () => call("get_startup_status"),
  setStartWithWindows: (enabled) => call("set_start_with_windows", { enabled }),
  exportConfig: () => call("export_config"),
  importConfigPreview: () => call("import_config_preview"),
  importConfigApply: (data, confirmationToken) =>
    call("import_config_apply", { input: { import: data, confirmationToken } }),
  restartAsAdmin: () => call("restart_as_administrator"),
};
