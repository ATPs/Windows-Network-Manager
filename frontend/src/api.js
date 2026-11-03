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
  ipSettings: (interfaceIndex) => call("get_ip_settings", { interfaceIndex }),
  previewIpSettings: (input) => call("preview_ip_settings", { input }),
  applyIpSettings: (settings, confirmationToken) =>
    call("apply_ip_settings", { input: { settings, confirmationToken } }),
  previewNetworkCategory: (input) => call("preview_network_category", { input }),
  applyNetworkCategory: (category, confirmationToken) =>
    call("apply_network_category", { input: { category, confirmationToken } }),
  keepConfigChange: (revertId) => call("keep_config_change", { revertId }),
  revertConfigChange: (revertId) => call("revert_config_change", { revertId }),
  wifiProfileDetails: (adapterGuid) => call("get_wifi_profile_details", { adapterGuid }),
  previewWifiProfileChange: (input) => call("preview_wifi_profile_change", { input }),
  applyWifiProfileChange: (change, confirmationToken) =>
    call("apply_wifi_profile_change", { input: { change, confirmationToken } }),
  previewNetworkTrigger: (input) => call("preview_network_trigger", { input }),
  saveNetworkTrigger: (trigger, confirmationToken) =>
    call("save_network_trigger", { input: { trigger, confirmationToken } }),
  removeNetworkTrigger: (id) => call("remove_network_trigger", { id }),
  previewConnectNewWifi: (input) => call("preview_connect_new_wifi", { input }),
  connectNewWifi: (input, password) => call("connect_new_wifi", { input, password }),
  vpnConnections: () => call("get_vpn_connections"),
  previewVpnAction: (input) => call("preview_vpn_action", { input }),
  applyVpnAction: (vpn, confirmationToken) =>
    call("apply_vpn_action", { input: { vpn, confirmationToken } }),
  openVpnSettings: () => call("open_vpn_settings"),
  previewDomainRoute: (input) => call("preview_domain_route", { input }),
  applyDomainRoute: (plan, confirmationToken) =>
    call("apply_domain_route", { input: { plan, confirmationToken } }),
  hostsFile: () => call("get_hosts_file"),
  appRules: (othersDirect = false) => call("get_app_rules", { othersDirect }),
  saveAppRule: (rule) => call("save_app_rule", { rule }),
  removeAppRule: (id) => call("remove_app_rule", { id }),
  pickProgram: () => call("pick_program"),
  runningPrograms: () => call("list_running_programs"),
  checkLocalProxy: (port) => call("check_local_proxy", { port }),
  previewHostsChange: (input) => call("preview_hosts_change", { input }),
  applyHostsChange: (input, confirmationToken) =>
    call("apply_hosts_change", { input: { ...input, confirmationToken } }),
  restartAsAdmin: () => call("restart_as_administrator"),
  monitorHistory: (query) => call("get_monitor_history", { query }),
  monitorStorage: () => call("get_monitor_storage"),
  chooseMonitorFolder: () => call("choose_monitor_folder"),
  setMonitorFolder: (confirmationToken, copyExisting) =>
    call("set_monitor_folder", { input: { confirmationToken, copyExisting } }),
  resetMonitorFolder: () => call("reset_monitor_folder"),
  clearMonitorHistory: () => call("clear_monitor_history"),
  openMonitorFolder: () => call("open_monitor_folder"),
  backups: () => call("list_network_backups"),
  createBackup: (name) => call("create_network_backup", { input: { name } }),
  renameBackup: (id, name) => call("rename_network_backup", { input: { id, name } }),
  keepBackup: (id) => call("keep_network_backup", { input: { id } }),
  deleteBackup: (id) => call("delete_network_backup", { input: { id } }),
  previewBackupRestore: (id, include) => call("preview_backup_restore", { input: { id, include } }),
  applyBackupRestore: (id, include, confirmationToken) =>
    call("apply_backup_restore", { input: { id, include, confirmationToken, acknowledged: true } }),
};
