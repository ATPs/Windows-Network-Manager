import { LitElement, html, nothing } from "lit";
import { api } from "./api.js";
import { copy, messages } from "./i18n.js";
import {
  autoGuidKey,
  autoResultText,
  compareRoutes,
  errorMessage,
  formatRelative,
  formatTime,
  interpolate,
  isAdapterDisabled,
  isAdapterOnline,
  routeIdentity,
} from "./format.js";
import { icon } from "./icons.js";
import { formatByteRate, formatRate } from "./charts.js";
import { spinner } from "./ui.js";
import { renderOverview } from "./views/overview.js";
import { renderAdapters } from "./views/adapters.js";
import { renderRoutes, renderRouteDrawer } from "./views/routes.js";
import { renderProfiles } from "./views/profiles.js";
import { renderDiagnostics } from "./views/diagnostics.js";
import { renderTools } from "./views/tools.js";
import { renderMonitor } from "./views/monitor.js";
import { renderIpDrawer } from "./views/ip-editor.js";
import { renderHosts } from "./views/hosts.js";
import { renderApps, normalizeProgramEntry, appRuleErrors } from "./views/apps.js";
import { renderMaintenance } from "./views/maintenance.js";
import { FONT_SIZES, renderSettings } from "./views/settings.js";

export const views = [
  "overview",
  "adapters",
  "routes",
  "profiles",
  "diagnostics",
  "tools",
  "apps",
  "monitor",
  "hosts",
  "maintenance",
  "settings",
];
const viewRenderers = {
  overview: renderOverview,
  adapters: renderAdapters,
  routes: renderRoutes,
  profiles: renderProfiles,
  diagnostics: renderDiagnostics,
  tools: renderTools,
  apps: renderApps,
  monitor: renderMonitor,
  hosts: renderHosts,
  maintenance: renderMaintenance,
  settings: renderSettings,
};
const TOAST_TIMEOUT = { success: 4000, info: 5000 };
const MONITOR_POLL_MS = 2000;
const HISTORY_POLL_MS = 60000;
export const MONITOR_RANGES = ["live", "1h", "24h", "7d", "30d"];
export const defaultMonitorCards = { traffic: true, latency: true, signal: true, daily: true, events: true, notifications: true };
export const defaultMonitorSettings = {
  historyEnabled: true,
  retentionDays: 30,
  latencyEnabled: true,
  latencyIntervalSeconds: 10,
  latencyPublicDns: false,
  folder: null,
};
export const defaultNotifications = {
  enabled: false,
  adapters: true,
  wifi: true,
  defaultRoute: true,
  tunnels: false,
};

const readStorage = (key) => {
  try {
    return globalThis.localStorage?.getItem(key) ?? null;
  } catch {
    return null;
  }
};
const writeStorage = (key, value) => {
  try {
    globalThis.localStorage?.setItem(key, value);
  } catch {
    // UI preferences are best effort.
  }
};
const readJson = (key) => {
  try {
    return JSON.parse(readStorage(key) || "null");
  } catch {
    return null;
  }
};
const isEditable = (target) =>
  target instanceof Element &&
  (target.isContentEditable ||
    ["INPUT", "SELECT", "TEXTAREA"].includes(target.tagName));

export class NetworkManagerApp extends LitElement {
  createRenderRoot() {
    return this;
  }

  constructor() {
    super();
    const savedView = readStorage("wnm-view");
    const routePrefs = readJson("wnm-route-filters") || {};
    this.view = views.includes(savedView) ? savedView : "overview";
    this.version = "";
    this.settings = {
      language: "en",
      theme: readStorage("wnm-theme") || "system",
      fontSize: "default",
      showSystemInterfaces: false,
      showAllRoutes: false,
      notifications: { ...defaultNotifications },
      monitor: { ...defaultMonitorSettings },
    };
    this.network = {
      overview: null,
      routes: null,
      refreshedAt: null,
      source: "live",
      stale: false,
    };
    this.config = {
      profiles: [],
      recentSnapshots: [],
      managedRoutes: [],
      networkTriggers: [],
      domainRoutes: [],
      hasBaseline: false,
    };
    this.privilege = { isAdmin: false };
    this.busy = new Set();
    this.wifiProfiles = new Map();
    this.wifiAuto = { adapters: [], rules: [], statuses: [], discoveryError: null };
    this.wifiAutoDrafts = new Map();
    this.wifiAutoEditing = new Set();
    this.wifiAutoLoading = null;
    this.wifiAutoEventUnlisten = null;
    this.message = null;
    this.toasts = [];
    this.toastSeq = 0;
    this.activity = [];
    this.activitySeq = 0;
    this.activityOpen = false;
    this.confirmation = null;
    this.nameDialog = null;
    this.routeSearch = "";
    this.routeFamily = ["IPv4", "IPv6", "all"].includes(routePrefs.family)
      ? routePrefs.family
      : "IPv4";
    this.routeSort = routePrefs.sort?.key
      ? routePrefs.sort
      : { key: "destination", dir: 1 };
    this.routeStaticOnly = false;
    this.routeDrawerOpen = false;
    this.routeFormVersion = 0;
    this.routeDraft = null;
    this.editingRoute = null;
    this.adapterSearch = "";
    this.adapterFilter = "all";
    this.expandedAdapters = new Set();
    this.defaultsSelection = null;
    this.defaultsRemoveRoutes = true;
    this.backups = null;
    this.backupTab = "manual";
    this.backupDetails = new Set();
    this.backupInclude = { ip: true, metric: true, category: true, routes: true, hosts: true };
    this.diagnostics = null;
    this.coexistence = null;
    this.tools = {
      connectivity: null,
      lookupHost: "",
      lookup: null,
      trace: null,
      connections: null,
      connectionFilters: { search: "", protocol: "all", state: "all" },
      wifiAdapter: "",
      wifiScan: null,
    };
    this.toolEventUnlisten = [];
    const monitorPrefs = readJson("wnm-monitor-prefs") || {};
    this.monitor = {
      snapshot: null,
      events: null,
      eventFilters: { kind: "all", tunnels: false },
      hover: null,
      range: MONITOR_RANGES.includes(monitorPrefs.range) ? monitorPrefs.range : "live",
      selected: Array.isArray(monitorPrefs.selected) ? monitorPrefs.selected.slice(0, 16) : [],
      cards: { ...defaultMonitorCards, ...(monitorPrefs.cards || {}) },
      rateUnit: monitorPrefs.rateUnit === "bytes" ? "bytes" : "bits",
      history: null,
      storage: null,
    };
    this.monitorTimer = null;
    this.historyTimer = null;
    this.startup = null;
    this.ipEditor = null;
    this.pendingRevert = null;
    this.revertTimer = null;
    this.wifiProfileDetails = new Map();
    this.wifiProfilesOpen = new Set();
    this.routeDraftFamily = "IPv4";
    this.triggerDraft = null;
    this.newWifi = null;
    this.vpn = null;
    this.domainDraft = { domain: "", nextHop: "", interfaceIndex: "", routeMetric: "5" };
    this.hosts = null;
    this.appRules = null;
    this.appRuleDraft = null;
    this.runningPrograms = null;
    this.portCheck = null;
    this.appRulesOthersDirect = readStorage("wnm-app-rules-others-direct") === "1";
    this.hostsEdit = null;
    this.refreshTimer = null;
    this.clockTimer = null;
    this.onVisibilityChange = () => {
      this.scheduleRefresh();
      this.syncMonitorPolling();
    };
    this.onKeydown = (event) => this.handleShortcut(event);
  }

  connectedCallback() {
    super.connectedCallback();
    document.addEventListener("visibilitychange", this.onVisibilityChange);
    document.addEventListener("keydown", this.onKeydown);
    this.clockTimer = setInterval(() => this.requestUpdate(), 10_000);
    this.initialize();
  }
  disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener("visibilitychange", this.onVisibilityChange);
    document.removeEventListener("keydown", this.onKeydown);
    clearTimeout(this.refreshTimer);
    clearInterval(this.clockTimer);
    clearInterval(this.revertTimer);
    this.stopMonitorPolling();
    if (this.wifiAutoEventUnlisten) {
      this.wifiAutoEventUnlisten();
      this.wifiAutoEventUnlisten = null;
    }
    this.toolEventUnlisten.forEach((unlisten) => unlisten());
    this.toolEventUnlisten = [];
  }

  t(key) {
    return copy[this.settings.language]?.[key] || copy.en[key] || key;
  }
  /** Formats a translated template such as "Showing {shown} of {total}". */
  tf(key, params) {
    return interpolate(this.t(key), params);
  }
  /** Translates a backend notice `{code, params, text}`, falling back to its English text. */
  msg(notice) {
    if (!notice) return "";
    if (typeof notice === "string") return notice;
    const table = messages[this.settings.language] || {};
    const params = { ...(notice.params || {}) };
    const variant = params.mode || params.action;
    const template = table[`${notice.code}:${variant}`] || table[notice.code];
    if (!template) return notice.text || notice.code || "";
    if (params.value === "automatic") params.value = this.t("automaticMetric");
    return interpolate(template, params);
  }
  warningsOf(preview) {
    if (Array.isArray(preview?.notices) && preview.notices.length)
      return preview.notices.map((notice) => this.msg(notice));
    return preview?.warnings || [];
  }
  summaryOf(result) {
    if (!result) return "";
    const code = result.summaryCode || result.messageCode;
    if (code) return this.msg(code);
    return result.message || result.summary || "";
  }
  isBusy(key) {
    return this.busy.has(key);
  }
  isBusyPrefix(prefix) {
    return [...this.busy].some((key) => key.startsWith(prefix));
  }
  setBusy(key, value) {
    value ? this.busy.add(key) : this.busy.delete(key);
    this.requestUpdate();
  }

  async initialize() {
    try {
      this.subscribeWifiAutoEvents();
      this.subscribeToolEvents();
      this.loadVersion();
      const bootstrap = await api.bootstrap();
      this.settings = { ...this.settings, ...(bootstrap.settings || {}) };
      this.config = { ...this.config, ...(bootstrap.config || bootstrap) };
      this.privilege = bootstrap.privilege || this.privilege;
      if (bootstrap.cachedNetworkState)
        this.applyNetwork(bootstrap.cachedNetworkState);
      this.settings.notifications = { ...defaultNotifications, ...(this.settings.notifications || {}) };
      this.settings.monitor = { ...defaultMonitorSettings, ...(this.settings.monitor || {}) };
      this.applyTheme();
      this.requestUpdate();
      this.syncMonitorPolling();
      this.loadStartupStatus();
      this.loadVpnConnections();
      this.refreshConfig().catch(() => {});
      await this.refresh(["interfaces", "routes"]);
      this.scheduleRefresh();
    } catch (error) {
      this.notify(errorMessage(error), "error");
    }
  }

  async loadVersion() {
    try {
      const { getVersion } = await import("@tauri-apps/api/app");
      const version = await getVersion();
      if (typeof version === "string") this.version = version;
      this.requestUpdate();
    } catch {
      // The version is unavailable outside the Tauri runtime.
    }
  }

  applyTheme() {
    document.documentElement.lang = this.settings.language;
    document.documentElement.dataset.theme = this.settings.theme;
    writeStorage("wnm-theme", this.settings.theme);
  }

  applyNetwork(result) {
    this.network = {
      ...this.network,
      overview: result.overview ?? this.network.overview,
      routes: result.routes ?? this.network.routes,
      refreshedAt:
        result.refreshedAt ||
        result.overview?.refreshedAt ||
        result.routes?.refreshedAt ||
        this.network.refreshedAt,
      source: result.source || "live",
      stale: Boolean(result.stale),
    };
    if (result.coexistence) this.coexistence = result.coexistence;
  }

  scheduleRefresh() {
    clearTimeout(this.refreshTimer);
    if (document.hidden) return;
    this.refreshTimer = setTimeout(async () => {
      await this.refresh(["interfaces", "routes"], false, true);
      this.scheduleRefresh();
    }, 15_000);
  }

  isRefreshing() {
    return this.isBusyPrefix("refresh:");
  }

  async refresh(scopes = ["interfaces", "routes"], force = false, quiet = false) {
    const key = `refresh:${scopes.sort().join(",")}`;
    if (this.isBusy(key)) return;
    this.setBusy(key, true);
    try {
      this.applyNetwork(await api.refresh(scopes, force));
      this.loadWifiProfiles();
      await this.loadWifiAutoStatus();
    } catch (error) {
      if (!quiet) this.notify(errorMessage(error), "error");
    } finally {
      this.setBusy(key, false);
    }
  }

  async loadWifiProfiles() {
    const adapters = this.network.overview?.interfaces || [];
    for (const adapter of adapters.filter(
      (item) => item.wifi && !this.wifiProfiles.has(item.alias),
    )) {
      this.wifiProfiles.set(adapter.alias, null);
      this.requestUpdate();
      try {
        const result = await api.wifiProfiles(adapter.alias);
        this.wifiProfiles.set(adapter.alias, result.profiles || []);
      } catch {
        this.wifiProfiles.set(adapter.alias, []);
      }
      this.requestUpdate();
    }
  }

  async subscribeWifiAutoEvents() {
    if (this.wifiAutoEventUnlisten) return;
    try {
      const { listen } = await import("@tauri-apps/api/event");
      this.wifiAutoEventUnlisten = await listen(
        "wifi-auto-connect-updated",
        () => this.loadWifiAutoStatus(),
      );
    } catch {
      // The event bridge is unavailable in browser and unit-test environments.
    }
  }

  async subscribeToolEvents() {
    if (this.toolEventUnlisten.length) return;
    try {
      const { listen } = await import("@tauri-apps/api/event");
      this.toolEventUnlisten = [
        await listen("trace-hop", (event) => this.onTraceHop(event.payload)),
        await listen("trace-done", (event) => this.onTraceDone(event.payload)),
        await listen("network-event", (event) => this.onNetworkEvents(event.payload)),
        await listen("config-reverted", (event) => this.onConfigReverted(event.payload)),
      ];
    } catch {
      // The event bridge is unavailable in browser and unit-test environments.
    }
  }

  async loadWifiAutoStatus() {
    if (this.wifiAutoLoading) return this.wifiAutoLoading;
    this.wifiAutoLoading = api
      .wifiAutoStatus()
      .then((status) => {
        this.wifiAuto = {
          adapters: status?.adapters || [],
          rules: status?.rules || [],
          statuses: status?.statuses || [],
          discoveryError: status?.discoveryError || null,
        };
        this.requestUpdate();
        return this.wifiAuto;
      })
      .catch(() => {
        // Keep the last known status while the app is starting or the bridge is unavailable.
        return this.wifiAuto;
      })
      .finally(() => {
        this.wifiAutoLoading = null;
      });
    return this.wifiAutoLoading;
  }

  async updateSettings(patch) {
    const previous = this.settings;
    this.settings = { ...this.settings, ...patch };
    this.applyTheme();
    this.requestUpdate();
    try {
      this.settings = {
        ...this.settings,
        ...(await api.updateSettings(this.settings)),
      };
    } catch (error) {
      this.settings = previous;
      this.applyTheme();
      this.notify(errorMessage(error), "error");
    }
  }

  async refreshConfig() {
    this.config = { ...this.config, ...(await api.profiles()) };
    this.requestUpdate();
  }
  async restartAsAdmin() {
    await this.run("elevation", () => api.restartAsAdmin(), [], this.t("restartAdmin"));
  }

  // ---- Feedback -----------------------------------------------------------

  notify(text, kind = "info") {
    this.message = { text, kind };
    const id = ++this.toastSeq;
    this.toasts = [...this.toasts, { id, text, kind }].slice(-4);
    if (TOAST_TIMEOUT[kind])
      setTimeout(() => this.dismissToast(id), TOAST_TIMEOUT[kind]);
    this.requestUpdate();
  }
  dismissToast(id) {
    this.toasts = this.toasts.filter((toast) => toast.id !== id);
    if (!this.toasts.length) this.message = null;
    this.requestUpdate();
  }
  dismissMessage() {
    this.toasts = [];
    this.message = null;
    this.requestUpdate();
  }

  async copyText(text) {
    try {
      await navigator.clipboard.writeText(String(text));
      this.notify(this.t("copied"), "success");
    } catch (error) {
      this.notify(`${this.t("copyFailed")}: ${errorMessage(error)}`, "error");
    }
  }

  activityLabel(key) {
    const labels = [
      ["route:add", "actRouteAdd"],
      ["route:edit", "actRouteEdit"],
      ["route:persist", "actRoutePersist"],
      ["route:remove", "actRouteRemove"],
      ["metric:", "actMetric"],
      ["adapter:", "actAdapter"],
      ["wifi-auto:", "wifiAutoConnect"],
      ["wifi:", "actWifi"],
      ["snapshot", "saveSnapshot"],
      ["recent:", "actRestore"],
      ["delete-recent:", "actDelete"],
      ["delete:", "actDelete"],
      ["profile:", "actRestore"],
      ["profile", "saveProfile"],
      ["reset", "undoLastChange"],
      ["defaults", "restoreDefaults"],
      ["elevation", "restartAdmin"],
      ["fix:flushDns", "flushDns"],
      ["fix:renewDhcp", "renewDhcp"],
      ["fix:restartAdapter", "restartAdapter"],
      ["startup", "startWithWindows"],
      ["import", "importConfig"],
      ["ip:", "editIpSettings"],
      ["category:", "networkCategory"],
      ["wifi-profile:", "savedNetworks"],
      ["trigger", "automaticConfigs"],
      ["wifi-new", "connectNewWifi"],
      ["vpn:", "vpnConnections"],
      ["domain:", "domainRoutes"],
      ["hosts", "hosts"],
      ["apps:", "apps"],
      ["revert:keep", "keepSettings"],
      ["revert:now", "revertNow"],
    ];
    const match = labels.find(([prefix]) => key.startsWith(prefix));
    return match ? this.t(match[1]) : key;
  }

  /** Runs a write operation: busy state, toast, activity entry and refresh. */
  async run(key, task, scopes = ["interfaces", "routes"], label = null) {
    if (this.isBusy(key)) return;
    this.setBusy(key, true);
    const entry = {
      id: ++this.activitySeq,
      label: label || this.activityLabel(key),
      time: new Date().toISOString(),
      status: "running",
      message: "",
      result: null,
    };
    this.activity = [entry, ...this.activity].slice(0, 50);
    try {
      const result = await task();
      const failed =
        JSON.stringify(result || {}).includes('"status":"failed"') ||
        result?.partial;
      const text =
        this.summaryOf(result) ||
        (failed ? this.t("operationPartial") : this.t("operationComplete"));
      Object.assign(entry, {
        status: failed ? "partial" : "success",
        message: text,
        result: result ?? null,
      });
      this.notify(text, failed ? "warning" : "success");
      if (scopes.length) await this.refresh(scopes, true, true);
      await this.refreshConfig().catch(() => {});
      return result;
    } catch (error) {
      Object.assign(entry, { status: "failed", message: errorMessage(error) });
      this.notify(errorMessage(error), "error");
      return null;
    } finally {
      this.setBusy(key, false);
    }
  }

  /** Runs a read-only request (preview, diagnostics): busy state and error toast only. */
  async preview(key, task) {
    if (this.isBusy(key)) return null;
    this.setBusy(key, true);
    try {
      return await task();
    } catch (error) {
      this.notify(errorMessage(error), "error");
      return null;
    } finally {
      this.setBusy(key, false);
    }
  }

  requestConfirmation(lines, confirm, destructive = true, options = {}) {
    this.confirmation = {
      lines: lines.filter(Boolean),
      confirm,
      destructive,
      title: options.title || this.t("confirm"),
      rows: options.rows || [],
      note: options.note || "",
      confirmLabel: options.confirmLabel || this.t("confirm"),
      acknowledge: options.acknowledge || "",
      acknowledged: false,
      typeToConfirm: options.typeToConfirm || "",
      typed: "",
    };
    this.requestUpdate();
  }

  closeConfirmation() {
    this.confirmation = null;
    this.requestUpdate();
  }
  /** True once the extra acknowledgement or typed confirmation is satisfied. */
  confirmationReady() {
    const confirmation = this.confirmation;
    if (!confirmation) return false;
    if (confirmation.acknowledge && !confirmation.acknowledged) return false;
    if (confirmation.typeToConfirm && confirmation.typed.trim() !== confirmation.typeToConfirm) return false;
    return true;
  }

  async confirm() {
    if (!this.confirmationReady()) return;
    const action = this.confirmation?.confirm;
    this.closeConfirmation();
    if (action) await action();
  }

  // ---- Navigation and shortcuts ------------------------------------------

  setView(view) {
    if (!views.includes(view)) return;
    this.view = view;
    writeStorage("wnm-view", view);
    this.syncMonitorPolling();
    if (view === "hosts" && !this.hosts) this.loadHosts();
    if (view === "apps" && !this.appRules) this.loadAppRules();
    if (view === "maintenance") this.loadBackups();
    if (view === "settings") this.loadMonitorStorage();
    this.requestUpdate();
    this.updateComplete.then(() =>
      this.querySelector("main")?.scrollTo?.({ top: 0 }),
    );
  }

  saveRoutePrefs() {
    writeStorage(
      "wnm-route-filters",
      JSON.stringify({ family: this.routeFamily, sort: this.routeSort }),
    );
  }

  anyDialogOpen() {
    return Boolean(
      this.confirmation ||
        this.nameDialog ||
        this.routeDrawerOpen ||
        this.activityOpen ||
        this.ipEditor ||
        this.pendingRevert ||
        this.newWifi,
    );
  }

  handleShortcut(event) {
    if (event.key === "F5" || (event.ctrlKey && event.key.toLowerCase() === "r")) {
      event.preventDefault();
      this.refresh(["interfaces", "routes"], true);
      return;
    }
    if (this.anyDialogOpen()) return;
    if (event.ctrlKey && !event.altKey && ["=", "+", "-", "0"].includes(event.key)) {
      event.preventDefault();
      this.stepFontSize(event.key === "0" ? 0 : event.key === "-" ? -1 : 1);
      return;
    }
    if (event.ctrlKey && !event.altKey && /^[1-9]$/.test(event.key)) {
      event.preventDefault();
      this.setView(views[Number(event.key) - 1]);
      return;
    }
    if (event.key === "/" && !isEditable(event.target)) {
      const search = this.querySelector("[data-page-search]");
      if (search) {
        event.preventDefault();
        search.focus();
      }
    }
  }

  /** Moves the text size one step (+1/-1), or back to default (0). */
  stepFontSize(step) {
    const current = Math.max(0, FONT_SIZES.indexOf(this.settings.fontSize));
    const next = step === 0 ? "default" : FONT_SIZES[Math.min(FONT_SIZES.length - 1, Math.max(0, current + step))];
    if (next !== this.settings.fontSize) this.updateSettings({ fontSize: next });
  }

  async goToAdapter(item) {
    const key = String(item.interfaceIndex);
    this.expandedAdapters.add(key);
    this.adapterFilter = "all";
    this.adapterSearch = "";
    this.setView("adapters");
    await this.updateComplete;
    this.querySelector(`[data-adapter="${key}"]`)?.scrollIntoView({
      behavior: "smooth",
      block: "start",
    });
  }

  goToAdapters(filter = "all") {
    this.adapterFilter = filter;
    this.adapterSearch = "";
    this.setView("adapters");
  }

  goToRoutes({ staticOnly = false, search = "" } = {}) {
    this.routeStaticOnly = staticOnly;
    this.routeSearch = search;
    if (staticOnly || search) this.routeFamily = "all";
    this.setView("routes");
  }

  // ---- Data helpers -------------------------------------------------------

  interfaces() {
    const all = this.network.overview?.interfaces || [];
    return all.filter(
      (item) => this.settings.showSystemInterfaces || !item.systemInterface,
    );
  }

  managedRouteKeys() {
    return new Set(
      (this.config.managedRoutes || []).map((route) =>
        routeIdentity({ addressFamily: "IPv4", ...route }),
      ),
    );
  }

  isStaticRoute(route) {
    return Boolean(route.persistent || route.protocol === "NetMgmt");
  }

  routeRows() {
    const routes = this.network.routes?.routes || [];
    const search = this.routeSearch.trim().toLowerCase();
    const { key, dir } = this.routeSort;
    return routes
      .filter(
        (route) =>
          (this.routeFamily === "all" ||
            route.addressFamily === this.routeFamily) &&
          (this.settings.showAllRoutes || !route.hiddenByDefault) &&
          (!this.routeStaticOnly || this.isStaticRoute(route)) &&
          `${route.destinationPrefix} ${route.nextHop} ${route.interfaceAlias}`
            .toLowerCase()
            .includes(search),
      )
      .sort((a, b) => compareRoutes(a, b, key) * dir || compareRoutes(a, b, "destination"));
  }

  connectionLabel(item) {
    if (isAdapterDisabled(item)) return this.t("disabled");
    return isAdapterOnline(item) ? this.t("connected") : this.t("available");
  }

  connectivityInfo(item) {
    const value = String(item?.ipv4Connectivity || item?.ipv6Connectivity || "");
    const ipv6Internet = String(item?.ipv6Connectivity || "") === "Internet";
    if (String(item?.ipv4Connectivity || "") === "Internet" || ipv6Internet)
      return { kind: "success", text: this.t("connInternet"), icon: "globe" };
    if (["LocalNetwork", "Subnet"].includes(value))
      return { kind: "warning", text: this.t("connLocal"), icon: "warning" };
    if (["NoTraffic", "Disconnected"].includes(value))
      return { kind: "neutral", text: this.t("connNone"), icon: "info" };
    return null;
  }

  // ---- Metric, Wi-Fi and adapter actions ---------------------------------

  async applyMetric(item, family, key) {
    const id = `metric-${item.interfaceIndex}-${key}`;
    const busyKey = `metric:${item.interfaceIndex}:${family}`;
    const automatic = this.querySelector(`#${id}-auto`)?.checked;
    const value = this.querySelector(`#${id}-value`)?.value;
    const input = {
      interfaceIndex: item.interfaceIndex,
      addressFamily: family,
      automatic,
      metric: value,
    };
    const preview = await this.preview(busyKey, () => api.previewMetric(input));
    if (!preview) return;
    if (preview.unchanged) return this.notify(this.t("metricUnchanged"));
    const current = item.metrics?.[key];
    const describe = (auto, metric) =>
      auto ? `${this.t("automaticMetric")} (${metric ?? "—"})` : String(metric);
    this.requestConfirmation(
      [`${item.alias} · ${family}`, ...this.warningsOf(preview)],
      () =>
        this.run(
          busyKey,
          () => api.applyMetric(preview.input, preview.confirmationToken),
          ["interfaces", "routes"],
          `${this.t("actMetric")} · ${item.alias} ${family}`,
        ),
      true,
      {
        title: this.t("confirmMetricTitle"),
        rows: [
          {
            label: `${family} ${this.t("metric")}`,
            before: describe(current?.automaticMetric, current?.interfaceMetric),
            after: automatic ? this.t("automaticMetric") : String(value),
          },
        ],
      },
    );
  }

  async connectWifi(item) {
    const profileName = this.querySelector(`#wifi-${item.interfaceIndex}`)?.value;
    if (!profileName) return;
    const preview = await this.preview(`wifi-preview:${item.interfaceIndex}`, () =>
      api.previewWifi({ interfaceName: item.alias, profileName, action: "connect" }),
    );
    if (preview)
      this.requestConfirmation(
        [`${item.alias} · ${profileName}`, this.t("wifiConnectWarning"), ...this.warningsOf(preview)],
        () =>
          this.run(
            `wifi:${item.interfaceIndex}`,
            () => api.connectWifi(preview.wifi, preview.confirmationToken),
            ["interfaces"],
            `${this.t("connect")} · ${item.alias} → ${profileName}`,
          ),
        false,
        {
          title: this.t("confirmWifiTitle"),
          confirmLabel: this.t("connect"),
          rows: [
            {
              label: item.alias,
              before: item.wifi?.ssid || this.t("disconnected"),
              after: profileName,
            },
          ],
        },
      );
  }

  async disconnectWifi(item) {
    const preview = await this.preview(`wifi-preview:${item.interfaceIndex}`, () =>
      api.previewWifi({ interfaceName: item.alias, action: "disconnect" }),
    );
    if (preview)
      this.requestConfirmation(
        [`${item.alias} · ${this.t("disconnect")}`, ...this.warningsOf(preview)],
        () =>
          this.run(
            `wifi:${item.interfaceIndex}`,
            () => api.disconnectWifi(preview.wifi, preview.confirmationToken),
            ["interfaces"],
            `${this.t("disconnect")} · ${item.alias}`,
          ),
        true,
        { title: this.t("confirmDisconnectTitle"), confirmLabel: this.t("disconnect") },
      );
  }

  async adapterAction(item, action) {
    const key = `adapter:${item.interfaceIndex}`;
    const preview = await this.preview(key, () =>
      api.previewAdapter({ interfaceIndex: item.interfaceIndex, action }),
    );
    if (preview)
      this.requestConfirmation(
        [`${item.alias} (#${item.interfaceIndex})`, this.tf("powerOnlyThis", { name: item.alias }), ...this.warningsOf(preview)],
        () =>
          this.run(
            key,
            () =>
              api.applyAdapter(
                { interfaceIndex: item.interfaceIndex, action },
                preview.confirmationToken,
              ),
            ["interfaces", "routes"],
            `${this.t(action)} · ${item.alias}`,
          ),
        true,
        {
          title: this.tf(action === "enable" ? "confirmEnableTitle" : "confirmDisableTitle", { name: item.alias }),
          confirmLabel: this.tf(action === "enable" ? "enableNamed" : "disableNamed", { name: item.alias }),
        },
      );
  }

  // ---- Wi-Fi auto-connect -------------------------------------------------

  wifiAutoGuid(item, adapterInfo = null) {
    const matchingAdapter = item?.alias
      ? (this.wifiAuto.adapters || []).find(
          (adapter) =>
            String(adapter.interfaceName || "").toLowerCase() ===
            String(item.alias).toLowerCase(),
        )
      : null;
    return (
      item?.guid ||
      item?.adapterGuid ||
      item?.wifi?.guid ||
      item?.wifi?.adapterGuid ||
      adapterInfo?.adapterGuid ||
      matchingAdapter?.adapterGuid ||
      ""
    );
  }

  wifiAutoAdapter(guid) {
    const key = autoGuidKey(guid);
    return (this.wifiAuto.adapters || []).find(
      (adapter) => autoGuidKey(adapter.adapterGuid) === key,
    );
  }

  wifiAutoRule(guid) {
    const key = autoGuidKey(guid);
    return (this.wifiAuto.rules || []).find(
      (rule) => autoGuidKey(rule.adapterGuid) === key,
    );
  }

  wifiAutoStatus(guid) {
    const key = autoGuidKey(guid);
    return (this.wifiAuto.statuses || []).find(
      (status) => autoGuidKey(status.adapterGuid) === key,
    );
  }

  wifiAutoProfiles(guid, item, adapterInfo) {
    if (Array.isArray(adapterInfo?.profiles)) return adapterInfo.profiles;
    const profiles = this.wifiProfiles.get(item?.alias);
    return Array.isArray(profiles) ? profiles : [];
  }

  wifiAutoDraft(guid, item, adapterInfo) {
    const key = autoGuidKey(guid);
    const existing = this.wifiAutoDrafts.get(key);
    if (existing) return existing;
    const rule = this.wifiAutoRule(guid);
    const draft = {
      adapterGuid: rule?.adapterGuid || guid,
      interfaceName: item?.alias || adapterInfo?.interfaceName || "",
      profileName: rule?.profileName || item?.wifi?.currentProfile || "",
      intervalSeconds: Number(rule?.intervalSeconds) || 60,
      enabled: Boolean(rule?.enabled),
      fallbackProfiles: [...(rule?.fallbackProfiles || [])],
    };
    this.wifiAutoDrafts.set(key, draft);
    return draft;
  }

  updateWifiAutoDraft(guid, patch) {
    const key = autoGuidKey(guid);
    const current = this.wifiAutoDrafts.get(key) || {
      adapterGuid: guid,
      profileName: "",
      intervalSeconds: 60,
      enabled: false,
    };
    this.wifiAutoDrafts.set(key, { ...current, ...patch });
    this.requestUpdate();
  }

  toggleWifiAutoEditor(guid, open = !this.wifiAutoEditing.has(autoGuidKey(guid))) {
    const key = autoGuidKey(guid);
    open ? this.wifiAutoEditing.add(key) : this.wifiAutoEditing.delete(key);
    if (!open) this.wifiAutoDrafts.delete(key);
    this.requestUpdate();
  }

  wifiAutoRuleInput(draft) {
    return {
      adapterGuid: draft.adapterGuid,
      profileName: String(draft.profileName || ""),
      intervalSeconds: Number(draft.intervalSeconds),
      enabled: Boolean(draft.enabled),
      fallbackProfiles: (draft.fallbackProfiles || []).filter(
        (name, index, list) => name && name !== draft.profileName && list.indexOf(name) === index,
      ),
    };
  }

  setWifiAutoFallback(guid, slot, name) {
    const draft = this.wifiAutoDrafts.get(autoGuidKey(guid));
    if (!draft) return;
    const fallbacks = [...(draft.fallbackProfiles || [])];
    fallbacks[slot] = name;
    this.updateWifiAutoDraft(guid, { fallbackProfiles: fallbacks.filter((item, index) => item || index < slot) });
  }

  async saveWifiAuto(draft) {
    const rule = this.wifiAutoRuleInput(draft);
    if (!rule.adapterGuid || !rule.profileName)
      return this.notify(this.t("wifiAutoProfileRequired"), "error");
    if (!Number.isInteger(rule.intervalSeconds) || rule.intervalSeconds < 5 || rule.intervalSeconds > 86400)
      return this.notify(this.t("wifiAutoIntervalInvalid"), "error");
    const key = `wifi-auto:${autoGuidKey(rule.adapterGuid)}`;
    const preview = await this.preview(`${key}:preview`, () => api.previewWifiAuto(rule));
    if (!preview) return;
    this.requestConfirmation(
      [
        `${draft.interfaceName || rule.adapterGuid} · ${rule.profileName}`,
        this.t("wifiAutoConfirm"),
        ...this.warningsOf(preview),
      ],
      async () => {
        const result = await this.run(
          key,
          () => api.saveWifiAuto(preview.rule || rule, preview.confirmationToken),
          [],
          `${this.t("wifiAutoConnect")} · ${draft.interfaceName || ""}`,
        );
        if (result) {
          this.wifiAutoEditing.delete(autoGuidKey(rule.adapterGuid));
          await this.loadWifiAutoStatus();
        }
      },
      false,
      { title: this.t("confirmAutoTitle"), confirmLabel: this.t("save") },
    );
  }

  async disableWifiAuto(guid) {
    const key = `wifi-auto:${autoGuidKey(guid)}`;
    const result = await this.run(key, () => api.disableWifiAuto(guid), []);
    if (result) {
      const draft = this.wifiAutoDrafts.get(autoGuidKey(guid));
      if (draft) {
        this.wifiAutoDrafts.set(autoGuidKey(guid), { ...draft, enabled: false });
        this.requestUpdate();
      }
      await this.loadWifiAutoStatus();
    }
  }

  removeWifiAuto(guid, name = "") {
    this.requestConfirmation(
      [name || guid, this.t("wifiAutoRemoveHelp")],
      async () => {
        const key = `wifi-auto:${autoGuidKey(guid)}`;
        const result = await this.run(key, () => api.removeWifiAuto(guid), []);
        if (result) {
          this.wifiAutoDrafts.delete(autoGuidKey(guid));
          this.wifiAutoEditing.delete(autoGuidKey(guid));
          await this.loadWifiAutoStatus();
        }
      },
      true,
      { title: this.t("confirmRemoveAutoTitle"), confirmLabel: this.t("remove") },
    );
  }

  wifiAutoStateText(state) {
    const value = String(state || "").toLowerCase();
    const labels = {
      active: "wifiAutoActive",
      connected: "wifiAutoConnected",
      waiting: "wifiAutoWaiting",
      checking: "wifiAutoChecking",
      disabled: "wifiAutoDisabled",
      "development-disabled": "wifiAutoDevelopmentDisabled",
      development_disabled: "wifiAutoDevelopmentDisabled",
      connecting: "wifiAutoConnecting",
      error: "wifiAutoError",
      failed: "wifiAutoError",
    };
    return labels[value] ? this.t(labels[value]) : state || this.t("wifiAutoWaiting");
  }

  wifiAutoStateClass(state) {
    const value = String(state || "").toLowerCase();
    if (["active", "connected"].includes(value)) return "success";
    if (["error", "failed"].includes(value)) return "danger";
    if (["connecting", "checking"].includes(value)) return "info";
    return "neutral";
  }

  wifiAutoResultText(result) {
    const labels = {
      "already-connected": "wifiAutoResultAlreadyConnected",
      "request-sent": "wifiAutoResultRequested",
      connected: "wifiAutoResultConnected",
      "observe-failed": "wifiAutoResultCheckFailed",
      "connect-request-failed": "wifiAutoResultRequestFailed",
      "verification-timeout": "wifiAutoResultTimeout",
      "connected-fallback": "wifiAutoResultFallback",
      "on-fallback": "wifiAutoResultOnFallback",
    };
    return labels[result] ? this.t(labels[result]) : autoResultText(result);
  }

  activeAutoRuleCount() {
    return (this.wifiAuto.rules || []).filter((rule) => rule.enabled).length;
  }

  // ---- Routes -------------------------------------------------------------

  routeFormValues(form) {
    const data = new FormData(form);
    return {
      addressFamily: data.get("addressFamily") || "IPv4",
      destinationPrefix: data.get("destinationPrefix"),
      nextHop: data.get("nextHop"),
      interfaceIndex: data.get("interfaceIndex"),
      routeMetric: data.get("routeMetric"),
      persistent: data.get("persistent") === "on",
    };
  }

  async addRoute(event) {
    event.preventDefault();
    if (!this.privilege.isAdmin)
      return this.notify(this.t("permissionHelp"), "error");
    const input = this.routeFormValues(event.currentTarget);
    const original = this.editingRoute;
    const key = original ? "route:edit" : "route:add";
    const preview = await this.preview(`${key}:preview`, () =>
      original ? api.previewEditRoute(original, input) : api.previewRoute(input),
    );
    if (preview?.unchanged) return this.notify(this.t("routeUnchanged"));
    if (preview)
      this.requestConfirmation(
        [
          ...(original ? [this.routeDescription(preview.oldRoute)] : []),
          this.routeDescription(preview.route),
          ...this.warningsOf(preview),
        ],
        async () => {
          const result = await this.run(
            key,
            () =>
              original
                ? api.editRoute(preview.oldRoute, preview.route, preview.confirmationToken)
                : api.addRoute(preview.route, preview.confirmationToken),
            ["interfaces", "routes"],
            `${this.t(original ? "actRouteEdit" : "actRouteAdd")} · ${preview.route.destinationPrefix}`,
          );
          if (result) this.closeRouteDrawer();
        },
        true,
        { title: this.t(original ? "editRoute" : "addRoute") },
      );
  }

  routeDescription(route) {
    return `${route.destinationPrefix} → ${route.nextHop} · #${route.interfaceIndex} · ${this.t("metric")} ${route.routeMetric} · ${route.persistent ? this.t("persistent") : this.t("temporary")}`;
  }

  routeBusy() {
    return [...this.busy].some((key) => key.startsWith("route:"));
  }

  openRouteDrawer(route = null) {
    this.editingRoute = route ? { ...route } : null;
    this.routeDraftFamily = route?.addressFamily || (this.routeFamily === "IPv6" ? "IPv6" : "IPv4");
    this.routeDraft = route
      ? {
          destinationPrefix: route.destinationPrefix,
          nextHop: route.nextHop,
          interfaceIndex: String(route.interfaceIndex),
          routeMetric: String(route.routeMetric ?? 5),
        }
      : null;
    this.routeFormVersion += 1;
    this.routeDrawerOpen = true;
    this.requestUpdate();
  }

  closeRouteDrawer() {
    this.routeDrawerOpen = false;
    this.editingRoute = null;
    this.routeDraft = null;
    this.requestUpdate();
  }

  setRouteDraftFamily(family) {
    this.routeDraftFamily = family;
    this.routeDraft = null;
    this.routeFormVersion += 1;
    this.requestUpdate();
  }

  async startRouteEdit(route) {
    this.openRouteDrawer(route);
    await this.updateComplete;
    this.querySelector('[name="destinationPrefix"]')?.focus({ preventScroll: true });
  }

  async makePersistent(route) {
    const key = `route:persist:${route.destinationPrefix}:${route.interfaceIndex}:${route.nextHop}`;
    const preview = await this.preview(`${key}:preview`, () => api.previewPersistRoute(route));
    if (preview)
      this.requestConfirmation(
        [`${preview.route.destinationPrefix} → ${preview.route.nextHop}`, ...this.warningsOf(preview)],
        () =>
          this.run(
            key,
            () => api.persistRoute(preview.route, preview.confirmationToken),
            ["interfaces", "routes"],
            `${this.t("actRoutePersist")} · ${route.destinationPrefix}`,
          ),
        true,
        { title: this.t("makePersistent"), confirmLabel: this.t("makePersistent") },
      );
  }

  async removeRoute(route) {
    const key = `route:remove:${route.destinationPrefix}:${route.interfaceIndex}:${route.nextHop}`;
    const preview = await this.preview(`${key}:preview`, () => api.previewRemoveRoute(route));
    if (preview)
      this.requestConfirmation(
        [`${route.destinationPrefix} → ${route.nextHop}`, ...this.warningsOf(preview)],
        () =>
          this.run(
            key,
            () =>
              api.removeRoute(
                {
                  addressFamily: route.addressFamily,
                  destinationPrefix: route.destinationPrefix,
                  nextHop: route.nextHop,
                  interfaceIndex: route.interfaceIndex,
                  policyStores: route.policyStores,
                },
                preview.confirmationToken,
              ),
            ["interfaces", "routes"],
            `${this.t("actRouteRemove")} · ${route.destinationPrefix}`,
          ),
        true,
        { title: this.t("confirmRemoveRouteTitle"), confirmLabel: this.t("remove") },
      );
  }

  // ---- Backups ------------------------------------------------------------

  openNameDialog(kind, value = "", id = null) {
    this.nameDialog = { kind, value, id };
    this.requestUpdate();
  }
  closeNameDialog() {
    this.nameDialog = null;
    this.requestUpdate();
  }
  async saveNamed(event) {
    event.preventDefault();
    const name = new FormData(event.currentTarget).get("name")?.trim() || "";
    const { kind, id } = this.nameDialog;
    if (kind === "backup" || kind === "renameBackup") {
      this.closeNameDialog();
      return kind === "backup" ? this.createBackup(name) : this.renameBackup(id, name);
    }
    if (!name) return;
    this.closeNameDialog();
    if (kind === "snapshot")
      await this.run("snapshot", () => api.recentSnapshot(name), [
        "interfaces",
        "routes",
      ], `${this.t("saveSnapshot")} · ${name}`);
    else
      await this.run(
        "profile",
        () =>
          api.saveProfile({
            name,
            state: {
              overview: this.network.overview,
              routes: this.network.routes,
            },
          }),
        [],
        `${this.t("saveProfile")} · ${name}`,
      );
  }

  restoreChangeRows(changes = []) {
    return changes.flatMap((change) => {
      switch (change.kind) {
        case "ip": {
          const alias = change.alias || `#${change.interfaceIndex}`;
          return [
            change.target?.ipv4
              ? { label: `${alias} · IPv4`, before: this.describeIpv4(change.before?.ipv4), after: this.describeIpv4(change.target.ipv4) }
              : null,
            change.target?.dns
              ? { label: `${alias} · DNS`, before: this.describeDns(change.before?.dns), after: this.describeDns(change.target.dns) }
              : null,
          ].filter(Boolean);
        }
        case "category":
          return {
            label: `${change.alias} · ${this.t("networkCategory")}`,
            before: this.t(`category${change.before}`),
            after: this.t(`category${change.after}`),
          };
        case "hosts":
          return { label: this.t("hosts"), after: this.t("hostsReplaced") };
        case "metric": {
          const describe = (value) =>
            value?.automatic
              ? `${this.t("automaticMetric")}${value.metric != null ? ` (${value.metric})` : ""}`
              : String(value?.metric ?? "—");
          return {
            label: `${change.alias || `#${change.interfaceIndex}`} · ${change.addressFamily} ${this.t("metric")}`,
            before: describe(change.before),
            after: describe(change.after),
          };
        }
        case "routeAdd":
        case "routeRemove": {
          const route = change.route || {};
          return {
            label: this.t(change.kind === "routeAdd" ? "changeRouteAdd" : "changeRouteRemove"),
            after: `${route.destinationPrefix} → ${route.nextHop} · ${route.interfaceAlias || `#${route.interfaceIndex}`}`,
            kind: change.kind === "routeAdd" ? "add" : "remove",
          };
        }
        case "adapter":
          return {
            label: change.alias || `#${change.interfaceIndex}`,
            after: this.t(change.action === "disable" ? "disabled" : "enabled"),
            kind: change.action === "disable" ? "remove" : "add",
          };
        case "wifi":
          return {
            label: `${change.interfaceName} · Wi-Fi`,
            before: change.before || this.t("disconnected"),
            after: change.action === "disconnect" ? this.t("disconnected") : change.after || "—",
          };
        default:
          return { label: this.t("changeSkipped"), after: this.msg(change), kind: "skip" };
      }
    });
  }

  async restoreProfile(item, recent = false) {
    const key = `${recent ? "recent" : "profile"}:${item.id}`;
    const preview = await this.preview(`${key}:preview`, () =>
      recent ? api.recentPreview(item.id) : api.profilePreview(item.id),
    );
    if (!preview) return;
    const rows = this.restoreChangeRows(preview.changes);
    const hasDiff = Array.isArray(preview.changes);
    this.requestConfirmation(
      [
        item.name,
        hasDiff && !rows.length ? this.t("restoreNoDifferences") : this.summaryOf(preview),
        ...this.warningsOf(preview),
      ],
      () =>
        this.run(
          key,
          () =>
            recent
              ? api.recentRestore(item.id, preview.confirmationToken)
              : api.profileRestore(item.id, preview.confirmationToken),
          ["interfaces", "routes"],
          `${this.t("actRestore")} · ${item.name}`,
        ),
      true,
      {
        title: this.t("confirmRestoreTitle"),
        confirmLabel: this.t("restore"),
        rows,
        note: preview.unchangedCount
          ? this.tf("restoreUnchangedCount", { count: preview.unchangedCount })
          : "",
      },
    );
  }

  // ---- Network backups (Recovery) ---------------------------------------

  async loadBackups() {
    try {
      this.backups = await api.backups();
    } catch (error) {
      this.backups = this.backups || { backups: [], limits: { manual: 50, automatic: 20 } };
      this.notify(errorMessage(error), "error");
    }
    this.requestUpdate();
  }

  backupLabel(item) {
    if (item.name) return item.name;
    if (item.reason) return this.msg(item.reason);
    return this.t("unnamedBackup");
  }

  async createBackup(name) {
    const result = await this.run("backup:create", () => api.createBackup(name), [], `${this.t("createBackup")}${name ? ` · ${name}` : ""}`);
    if (result) await this.loadBackups();
  }

  async renameBackup(id, name) {
    const result = await this.run(`backup:rename:${id}`, () => api.renameBackup(id, name), [], this.t("renameBackup"));
    if (result) await this.loadBackups();
  }

  async keepBackup(item) {
    const result = await this.run(`backup:keep:${item.id}`, () => api.keepBackup(item.id), [], `${this.t("keepBackup")} · ${this.backupLabel(item)}`);
    if (result) await this.loadBackups();
  }

  deleteBackup(item) {
    this.requestConfirmation(
      [this.backupLabel(item), formatTime(item.createdAt, this.settings.language), this.t("deleteBackupHelp")],
      async () => {
        const result = await this.run(`backup:delete:${item.id}`, () => api.deleteBackup(item.id), [], `${this.t("actDelete")} · ${this.backupLabel(item)}`);
        if (result) await this.loadBackups();
      },
      true,
      { title: this.t("confirmDeleteTitle"), confirmLabel: this.t("delete") },
    );
  }

  toggleBackupDetails(item) {
    this.backupDetails.has(item.id) ? this.backupDetails.delete(item.id) : this.backupDetails.add(item.id);
    this.requestUpdate();
  }

  setBackupInclude(key, value) {
    this.backupInclude = { ...this.backupInclude, [key]: value };
    this.requestUpdate();
  }

  async restoreBackup(item) {
    const key = `backup:restore:${item.id}`;
    const include = { ...this.backupInclude };
    const preview = await this.preview(`${key}:preview`, () => api.previewBackupRestore(item.id, include));
    if (!preview) return;
    const skipped = (preview.skipped || []).map((skip) => ({ label: skip.alias || this.t("changeSkipped"), after: this.msg(skip), kind: "skip" }));
    if (!preview.changes?.length) {
      this.notify(this.t("backupNoDifferences"));
      return;
    }
    const label = `${this.t("restoreBackup")} · ${this.backupLabel(item)}`;
    this.requestConfirmation(
      [
        `${this.backupLabel(item)} · ${formatTime(item.createdAt, this.settings.language)}`,
        this.t("backupRestoreIntro"),
        ...this.warningsOf(preview),
      ],
      async () => {
        const result = await this.run(key, () => api.applyBackupRestore(item.id, include, preview.confirmationToken), ["interfaces", "routes"], label);
        if (result) {
          this.startRevertCountdown(result.revert, label);
          this.loadBackups();
        }
      },
      true,
      {
        title: this.t("confirmBackupRestoreTitle"),
        confirmLabel: this.t("restore"),
        rows: [...this.restoreChangeRows(preview.changes), ...skipped],
        note: preview.unchangedCount ? this.tf("restoreUnchangedCount", { count: preview.unchangedCount }) : "",
        acknowledge: this.t("backupRestoreAcknowledge"),
      },
    );
  }

  deleteProfile(item) {
    this.requestConfirmation(
      [item.name, this.t("deleteProfileHelp")],
      () => this.run(`delete:${item.id}`, () => api.deleteProfile(item.id), [], `${this.t("actDelete")} · ${item.name}`),
      true,
      { title: this.t("confirmDeleteTitle"), confirmLabel: this.t("delete") },
    );
  }

  deleteRecent(item) {
    this.requestConfirmation(
      [item.name, this.t("deleteRecentHelp")],
      () => this.run(`delete-recent:${item.id}`, () => api.deleteRecent(item.id), [], `${this.t("actDelete")} · ${item.name}`),
      true,
      { title: this.t("confirmDeleteTitle"), confirmLabel: this.t("delete") },
    );
  }

  // ---- Diagnostics and recovery ------------------------------------------

  async generateDiagnostics() {
    const report = await this.preview("diagnostics", () => api.diagnostics());
    if (report) {
      this.diagnostics = report;
      this.requestUpdate();
    }
  }

  async undoLastChange() {
    const preview = await this.preview("reset-preview", () => api.resetPreview());
    if (!preview) return;
    const items = Array.isArray(preview.scopeItems)
      ? preview.scopeItems.map((item) => this.msg(item))
      : Object.keys(preview.scope || {}).map((key) => `${this.t("affected")}: ${key}`);
    const baseline = preview.baseline;
    this.requestConfirmation(
      [
        baseline
          ? this.tf("baselineRecorded", {
              name: baseline.name || this.t("undoLastChange"),
              time: formatTime(baseline.createdAt, this.settings.language),
            })
          : "",
        ...items,
        ...this.warningsOf(preview),
      ],
      () => this.run("reset", () => api.reset(preview.confirmationToken), ["interfaces", "routes"], this.t("undoLastChange")),
      true,
      { title: this.t("undoLastChange"), confirmLabel: this.t("undo") },
    );
  }

  defaultsCandidates() {
    // Adapters owned by EasyTier, v2rayN and similar software are left to them.
    return this.interfaces().filter((item) => !item.systemInterface && !item.managedBy);
  }

  // ---- Coexistence with tunnel and proxy software -------------------------

  ownerLabel(owner) {
    return owner === "tunnel" ? this.t("tunnelSoftware") : owner || "";
  }

  managedLabel(owner) {
    return this.tf("managedBy", { owner: this.ownerLabel(owner) });
  }

  // ---- Tools (read-only) --------------------------------------------------

  openTools() {
    this.setView("tools");
    this.runConnectivityTest();
  }

  async runConnectivityTest() {
    const result = await this.preview("tools:connectivity", () => api.connectivityTest());
    if (result) {
      this.tools.connectivity = result;
      this.requestUpdate();
    }
  }

  async lookupRoute() {
    const host = this.tools.lookupHost.trim();
    if (!host) return;
    const result = await this.preview("tools:lookup", () => api.lookupRoute(host));
    if (result) {
      this.tools.lookup = result;
      this.requestUpdate();
    }
  }

  async startTrace() {
    const host = this.tools.lookupHost.trim();
    if (!host || this.isBusy("tools:trace")) return;
    const started = await this.preview("tools:trace-start", () => api.startTrace(host));
    if (!started) return;
    this.tools.trace = { ...started, hops: [], done: false };
    this.setBusy("tools:trace", true);
  }

  async cancelTrace() {
    try {
      await api.cancelTrace();
    } catch {
      // The trace finishes on its own when cancellation is unavailable.
    }
  }

  onTraceHop(hop) {
    const trace = this.tools.trace;
    if (!trace || hop?.traceId !== trace.traceId) return;
    trace.hops = [...trace.hops, hop];
    this.requestUpdate();
  }

  onTraceDone(done) {
    const trace = this.tools.trace;
    if (!trace || done?.traceId !== trace.traceId) return;
    Object.assign(trace, {
      done: true,
      reached: Boolean(done.reached),
      cancelled: Boolean(done.cancelled),
      error: done.error || null,
    });
    this.setBusy("tools:trace", false);
  }

  async loadConnections() {
    const result = await this.preview("tools:connections", () => api.activeConnections());
    if (result) {
      this.tools.connections = result;
      this.requestUpdate();
    }
  }

  setConnectionFilter(change) {
    this.tools.connectionFilters = { ...this.tools.connectionFilters, ...change };
    this.requestUpdate();
  }

  wifiAdapters() {
    return this.interfaces().filter((item) => item.wifi && item.guid);
  }

  async scanWifi(adapterGuid) {
    if (!adapterGuid) return;
    const result = await this.preview("tools:wifi", () => api.scanWifi(adapterGuid));
    if (result) {
      this.tools.wifiScan = result;
      this.requestUpdate();
    }
  }

  // ---- Quick fixes and app automation ------------------------------------

  async quickFix(action, item = null) {
    const key = item ? `fix:${action}:${item.interfaceIndex}` : `fix:${action}`;
    const preview = await this.preview(`${key}:preview`, () =>
      api.previewQuickFix({ action, interfaceIndex: item?.interfaceIndex ?? null }),
    );
    if (!preview) return;
    const adapterScoped = action !== "flushDns";
    this.requestConfirmation(
      [item ? `${item.alias} (#${item.interfaceIndex})` : "", ...this.warningsOf(preview)],
      () =>
        this.run(
          key,
          () => api.applyQuickFix(preview.fix, preview.confirmationToken),
          adapterScoped ? ["interfaces", "routes"] : [],
          `${this.t(action)}${item ? ` · ${item.alias}` : ""}`,
        ),
      adapterScoped,
      { title: this.t(`${action}Title`), confirmLabel: this.t(action) },
    );
  }

  async loadStartupStatus() {
    try {
      this.startup = await api.startupStatus();
      this.requestUpdate();
    } catch {
      // Startup status is unavailable outside the Tauri runtime.
    }
  }

  async setStartWithWindows(enabled) {
    const result = await this.run(
      "startup",
      () => api.setStartWithWindows(enabled),
      [],
      this.t("startWithWindows"),
    );
    if (result) {
      this.startup = result;
      this.requestUpdate();
    }
  }

  async exportConfig() {
    const result = await this.preview("export", () => api.exportConfig());
    if (result?.exported) this.notify(this.summaryOf(result), "success");
  }

  async importConfig() {
    const preview = await this.preview("import:preview", () => api.importConfigPreview());
    if (!preview || preview.cancelled) return;
    const summary = preview.summary || {};
    const rows = [
      summary.settings ? { label: this.t("settings"), after: this.t("importReplaced") } : null,
      summary.profilesAdded ? { label: this.t("savedProfiles"), after: this.tf("importAdded", { count: summary.profilesAdded }), kind: "add" } : null,
      summary.profilesReplaced ? { label: this.t("savedProfiles"), after: this.tf("importReplacedCount", { count: summary.profilesReplaced }) } : null,
      summary.rules ? { label: this.t("wifiAutoConnect"), after: this.tf("importRules", { count: summary.rules }) } : null,
    ].filter(Boolean);
    this.requestConfirmation(
      [preview.fileName || "", this.t("importHelp")],
      async () => {
        const result = await this.run(
          "import",
          () => api.importConfigApply(preview.import, preview.confirmationToken),
          [],
          this.t("importConfig"),
        );
        if (result?.settings) {
          this.settings = { ...this.settings, ...result.settings };
          this.settings.notifications = { ...defaultNotifications, ...(result.settings.notifications || {}) };
          this.applyTheme();
          await this.loadWifiAutoStatus();
          this.requestUpdate();
        }
      },
      false,
      { title: this.t("importConfigTitle"), confirmLabel: this.t("importConfig"), rows },
    );
  }

  // ---- Configuration editing (preview, confirm, keep-or-revert) ----------

  /** Starts the "Keep these settings?" countdown for an applied change. */
  startRevertCountdown(revert, label) {
    if (!revert?.revertId) return;
    this.pendingRevert = {
      revertId: revert.revertId,
      deadline: Date.now() + (revert.seconds || 60) * 1000,
      label,
    };
    clearInterval(this.revertTimer);
    this.revertTimer = setInterval(() => {
      if (!this.pendingRevert) clearInterval(this.revertTimer);
      this.requestUpdate();
    }, 1000);
    this.requestUpdate();
  }

  revertSecondsLeft(now = Date.now()) {
    if (!this.pendingRevert) return 0;
    return Math.max(0, Math.ceil((this.pendingRevert.deadline - now) / 1000));
  }

  clearPendingRevert() {
    this.pendingRevert = null;
    clearInterval(this.revertTimer);
    this.revertTimer = null;
    this.requestUpdate();
  }

  async keepChange() {
    const pending = this.pendingRevert;
    if (!pending) return;
    const result = await this.run("revert:keep", () => api.keepConfigChange(pending.revertId), [], `${this.t("keepSettings")} · ${pending.label}`);
    this.clearPendingRevert();
    if (!result) await this.refresh(["interfaces", "routes"], true, true);
  }

  async revertChange() {
    const pending = this.pendingRevert;
    if (!pending) return;
    this.clearPendingRevert();
    await this.run("revert:now", () => api.revertConfigChange(pending.revertId), ["interfaces", "routes"], `${this.t("revertNow")} · ${pending.label}`);
  }

  onConfigReverted(payload) {
    if (this.pendingRevert && payload?.revertId === this.pendingRevert.revertId) this.clearPendingRevert();
    this.notify(this.t("autoReverted"), "warning");
    this.refresh(["interfaces", "routes"], true, true);
  }

  canEditIp(item) {
    return Boolean(item && !item.managedBy && !item.systemInterface && item.metrics?.ipv4);
  }

  async openIpEditor(item) {
    this.ipEditor = { item, loading: true, draft: null, errors: {} };
    this.requestUpdate();
    const result = await this.preview(`ip:load:${item.interfaceIndex}`, () => api.ipSettings(item.interfaceIndex));
    if (!this.ipEditor || this.ipEditor.item !== item) return;
    if (!result) return this.closeIpEditor();
    const target = result.target || {};
    this.ipEditor = {
      item,
      loading: false,
      current: target,
      draft: {
        ipv4Mode: target.ipv4?.mode === "static" ? "static" : "dhcp",
        address: target.ipv4?.address || "",
        prefixLength: String(target.ipv4?.prefixLength ?? 24),
        gateway: target.ipv4?.gateway || "",
        dnsMode: target.dns?.mode === "manual" ? "manual" : "automatic",
        dnsServers: (target.dns?.servers || []).join(", "),
      },
    };
    this.requestUpdate();
  }

  updateIpDraft(change) {
    if (!this.ipEditor?.draft) return;
    this.ipEditor.draft = { ...this.ipEditor.draft, ...change };
    if (change.ipv4Mode === "static" && this.ipEditor.draft.dnsMode === "automatic")
      this.ipEditor.draft.dnsMode = "manual";
    this.requestUpdate();
  }

  closeIpEditor() {
    this.ipEditor = null;
    this.requestUpdate();
  }

  ipRequest(item, draft) {
    return {
      interfaceIndex: item.interfaceIndex,
      ipv4:
        draft.ipv4Mode === "static"
          ? { mode: "static", address: draft.address.trim(), prefixLength: Number(draft.prefixLength), gateway: draft.gateway.trim() }
          : { mode: "dhcp" },
      dns:
        draft.dnsMode === "manual"
          ? { mode: "manual", servers: draft.dnsServers.split(/[\s,;]+/).filter(Boolean) }
          : { mode: "automatic" },
    };
  }

  describeIpv4(value) {
    if (!value) return "—";
    if (value.mode === "dhcp") return this.t("ipAutomatic");
    return `${value.address || "—"}/${value.prefixLength ?? "—"}${value.gateway ? ` · ${this.t("gateway")} ${value.gateway}` : ` · ${this.t("noGateway")}`}`;
  }

  describeDns(value) {
    if (!value) return "—";
    return value.mode === "manual" ? (value.servers || []).join(", ") : this.t("dnsAutomatic");
  }

  async saveIpSettings() {
    const editor = this.ipEditor;
    if (!editor?.draft) return;
    const item = editor.item;
    const request = this.ipRequest(item, editor.draft);
    const preview = await this.preview(`ip:preview:${item.interfaceIndex}`, () => api.previewIpSettings(request));
    if (!preview) return;
    if (preview.settings?.unchanged) return this.notify(this.t("ipUnchanged"));
    const { target, before } = preview.settings;
    const rows = [
      target.ipv4 ? { label: `${item.alias} · IPv4`, before: this.describeIpv4(before.ipv4), after: this.describeIpv4(target.ipv4) } : null,
      target.dns ? { label: `${item.alias} · DNS`, before: this.describeDns(before.dns), after: this.describeDns(target.dns) } : null,
    ].filter(Boolean);
    this.requestConfirmation(
      [`${item.alias} (#${item.interfaceIndex})`, ...this.warningsOf(preview)],
      async () => {
        const result = await this.run(
          `ip:${item.interfaceIndex}`,
          () => api.applyIpSettings(request, preview.confirmationToken),
          ["interfaces", "routes"],
          `${this.t("editIpSettings")} · ${item.alias}`,
        );
        if (result) {
          this.closeIpEditor();
          this.startRevertCountdown(result.revert, `${this.t("editIpSettings")} · ${item.alias}`);
        }
      },
      true,
      {
        title: this.t("confirmIpTitle"),
        confirmLabel: this.t("apply"),
        rows,
        acknowledge: this.t("ipAcknowledge"),
      },
    );
  }

  async changeNetworkCategory(item, category) {
    const key = `category:${item.interfaceIndex}`;
    const input = { interfaceIndex: item.interfaceIndex, category };
    const preview = await this.preview(`${key}:preview`, () => api.previewNetworkCategory(input));
    if (!preview) return;
    if (preview.unchanged) return this.notify(this.t("categoryUnchanged"));
    this.requestConfirmation(
      [`${item.alias} (#${item.interfaceIndex})`, ...this.warningsOf(preview)],
      async () => {
        const result = await this.run(key, () => api.applyNetworkCategory(input, preview.confirmationToken), ["interfaces"], `${this.t("networkCategory")} · ${item.alias}`);
        if (result) this.startRevertCountdown(result.revert, `${this.t("networkCategory")} · ${item.alias}`);
      },
      true,
      {
        title: this.t("confirmCategoryTitle"),
        confirmLabel: this.t("apply"),
        rows: [{ label: item.alias, before: this.t(`category${preview.category.before}`), after: this.t(`category${category}`) }],
        acknowledge: this.t("categoryAcknowledge"),
      },
    );
  }

  async toggleWifiProfiles(item) {
    const guid = item.guid;
    if (this.wifiProfilesOpen.has(guid)) {
      this.wifiProfilesOpen.delete(guid);
      return this.requestUpdate();
    }
    this.wifiProfilesOpen.add(guid);
    await this.loadWifiProfileDetails(guid);
  }

  async loadWifiProfileDetails(guid) {
    this.wifiProfileDetails.set(guid, null);
    this.requestUpdate();
    const result = await this.preview(`wifi-profiles:${guid}`, () => api.wifiProfileDetails(guid));
    this.wifiProfileDetails.set(guid, result || { profiles: [] });
    this.requestUpdate();
  }

  async wifiProfileAction(item, profile, action, mode = null) {
    const key = `wifi-profile:${item.guid}:${profile.name}`;
    const input = { adapterGuid: item.guid, profileName: profile.name, action, mode };
    const preview = await this.preview(`${key}:preview`, () => api.previewWifiProfileChange(input));
    if (!preview) return;
    const forget = action === "forget";
    this.requestConfirmation(
      [`${item.alias} · ${profile.name}`, ...this.warningsOf(preview)],
      async () => {
        const result = await this.run(key, () => api.applyWifiProfileChange(input, preview.confirmationToken), ["interfaces"], `${this.t("savedNetworks")} · ${profile.name}`);
        if (result) {
          this.wifiProfiles.delete(item.alias);
          await this.loadWifiProfileDetails(item.guid);
          this.loadWifiProfiles();
        }
      },
      forget,
      {
        title: this.t(forget ? "confirmForgetTitle" : "confirmProfileTitle"),
        confirmLabel: this.t(forget ? "forget" : "apply"),
        typeToConfirm: forget ? profile.name : "",
      },
    );
  }

  // ---- Automatic configurations (network triggers) ----------------------

  openTriggerEditor() {
    const adapter = this.wifiAdapters()[0];
    this.triggerDraft = {
      adapterGuid: adapter?.guid || "",
      profileName: adapter?.wifi?.currentProfile || "",
      savedConfigId: (this.config.profiles || [])[0]?.id || "",
      enabled: true,
    };
    this.requestUpdate();
  }

  updateTriggerDraft(change) {
    this.triggerDraft = { ...this.triggerDraft, ...change };
    this.requestUpdate();
  }

  async saveTrigger(draft = this.triggerDraft) {
    if (!draft?.adapterGuid || !draft.profileName || !draft.savedConfigId)
      return this.notify(this.t("triggerIncomplete"), "error");
    const preview = await this.preview("trigger:preview", () => api.previewNetworkTrigger(draft));
    if (!preview) return;
    const config = (this.config.profiles || []).find((item) => item.id === draft.savedConfigId);
    this.requestConfirmation(
      [`${draft.profileName} → ${config?.name || draft.savedConfigId}`, ...this.warningsOf(preview)],
      async () => {
        const result = await this.run("trigger", () => api.saveNetworkTrigger(preview.trigger, preview.confirmationToken), [], this.t("automaticConfigs"));
        if (result) this.triggerDraft = null;
      },
      false,
      { title: this.t("confirmTriggerTitle"), confirmLabel: this.t("save"), acknowledge: this.t("triggerAcknowledge") },
    );
  }

  toggleTrigger(trigger, enabled) {
    return this.saveTrigger({ ...trigger, enabled });
  }

  removeTrigger(trigger) {
    this.requestConfirmation(
      [trigger.profileName, this.t("triggerRemoveHelp")],
      () => this.run(`trigger:remove:${trigger.id}`, () => api.removeNetworkTrigger(trigger.id), [], this.t("automaticConfigs")),
      true,
      { title: this.t("confirmRemoveTriggerTitle"), confirmLabel: this.t("remove") },
    );
  }

  // ---- Connect to a new Wi-Fi network -------------------------------------

  canConnectNew(ap) {
    return Boolean(ap?.ssid && !ap.savedProfile && !ap.connected && ["Open", "WPA2-Personal", "WPA3-Personal"].includes(ap.security));
  }

  async startConnectNew(ap) {
    const adapterGuid = this.tools.wifiScan?.adapterGuid;
    if (!adapterGuid || !this.canConnectNew(ap)) return;
    const network = { adapterGuid, ssid: ap.ssid, security: ap.security };
    const preview = await this.preview(`wifi-new:preview:${ap.ssid}`, () => api.previewConnectNewWifi(network));
    if (!preview) return;
    this.newWifi = { network: preview.network, preview, password: "", show: false };
    this.requestUpdate();
  }

  closeNewWifi() {
    if (this.newWifi) this.newWifi.password = "";
    this.newWifi = null;
    this.requestUpdate();
  }

  async submitNewWifi() {
    const dialog = this.newWifi;
    if (!dialog) return;
    const open = dialog.network.security === "Open";
    const password = open ? "" : dialog.password;
    if (!open && (password.length < 8 || password.length > 64)) return this.notify(this.t("wifiPasswordInvalid"), "error");
    // The password leaves the UI state before the request is sent.
    dialog.password = "";
    this.newWifi = null;
    this.requestUpdate();
    await this.run(
      "wifi-new",
      () => api.connectNewWifi({ ...dialog.network, confirmationToken: dialog.preview.confirmationToken }, password),
      ["interfaces"],
      `${this.t("connectNewWifi")} · ${dialog.network.ssid}`,
    );
  }

  // ---- Built-in VPN -------------------------------------------------------

  async loadVpnConnections() {
    try {
      this.vpn = (await api.vpnConnections())?.connections || [];
    } catch {
      this.vpn = this.vpn || [];
    }
    this.requestUpdate();
  }

  async vpnAction(connection, action) {
    const key = `vpn:${connection.name}`;
    const input = { name: connection.name, action };
    const preview = await this.preview(`${key}:preview`, () => api.previewVpnAction(input));
    if (!preview) return;
    this.requestConfirmation(
      [connection.name, ...this.warningsOf(preview)],
      async () => {
        await this.run(key, () => api.applyVpnAction(input, preview.confirmationToken), ["interfaces", "routes"], `${this.t(action)} · ${connection.name}`);
        await this.loadVpnConnections();
      },
      action === "disconnect",
      { title: this.t(action === "connect" ? "confirmVpnConnectTitle" : "confirmVpnDisconnectTitle"), confirmLabel: this.t(action) },
    );
  }

  async openVpnSettings() {
    await this.preview("vpn:settings", () => api.openVpnSettings());
  }

  // ---- Domain routes ------------------------------------------------------

  updateDomainDraft(change) {
    this.domainDraft = { ...this.domainDraft, ...change };
    this.requestUpdate();
  }

  async domainRoute(request, mode = "save") {
    const input = { ...request, mode };
    const key = `domain:${request.domain}`;
    const preview = await this.preview(`${key}:preview`, () => api.previewDomainRoute(input));
    if (!preview) return;
    if (preview.unchanged) return this.notify(this.t("domainUnchanged"));
    const plan = preview.plan;
    const rows = [
      ...(plan.add || []).map((route) => ({ label: this.t("changeRouteAdd"), after: `${route.destinationPrefix} → ${route.nextHop}`, kind: "add" })),
      ...(plan.remove || []).map((route) => ({ label: this.t("changeRouteRemove"), after: `${route.destinationPrefix} → ${route.nextHop}`, kind: "remove" })),
    ];
    this.requestConfirmation(
      [plan.domain, ...this.warningsOf(preview)],
      async () => {
        const result = await this.run(key, () => api.applyDomainRoute(plan, preview.confirmationToken), ["routes"], `${this.t("domainRoutes")} · ${plan.domain}`);
        if (result && mode === "save") this.domainDraft = { ...this.domainDraft, domain: "" };
      },
      true,
      { title: this.t(mode === "remove" ? "confirmDomainRemoveTitle" : "confirmDomainTitle"), confirmLabel: this.t(mode === "remove" ? "remove" : "apply"), rows },
    );
  }

  // ---- Per-app rules (v2rayN) ---------------------------------------------

  async loadAppRules() {
    const result = await this.preview("apps:load", () => api.appRules(this.appRulesOthersDirect));
    if (result) {
      this.appRules = result;
      this.requestUpdate();
    }
  }

  setAppRulesOthersDirect(enabled) {
    this.appRulesOthersDirect = Boolean(enabled);
    writeStorage("wnm-app-rules-others-direct", enabled ? "1" : "0");
    this.loadAppRules();
  }

  newAppRule() {
    const id = `rule-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
    this.appRuleDraft = {
      id,
      name: "",
      programs: [],
      target: "proxy",
      localProxy: { protocol: "socks", host: ["127", "0", "0", "1"].join("."), port: "" },
      enabled: true,
      entry: "",
      isNew: true,
    };
    this.portCheck = null;
    this.requestUpdate();
  }

  editAppRule(rule) {
    this.appRuleDraft = {
      ...rule,
      programs: [...rule.programs],
      localProxy: rule.localProxy
        ? { ...rule.localProxy, port: String(rule.localProxy.port) }
        : { protocol: "socks", host: ["127", "0", "0", "1"].join("."), port: "" },
      entry: "",
      isNew: false,
    };
    this.portCheck = null;
    this.requestUpdate();
  }

  closeAppRuleDraft() {
    this.appRuleDraft = null;
    this.portCheck = null;
    this.requestUpdate();
  }

  updateAppRuleDraft(change) {
    if (!this.appRuleDraft) return;
    this.appRuleDraft = { ...this.appRuleDraft, ...change };
    if (change.localProxy) this.portCheck = null;
    this.requestUpdate();
  }

  /** Adds a program entry; a typed file name takes the exact case of a running program. */
  addAppRuleProgram(entry) {
    const draft = this.appRuleDraft;
    let program = normalizeProgramEntry(entry);
    if (!draft || !program) return false;
    if (!program.includes("/")) {
      const running = (this.runningPrograms || []).find((item) => item.name.toLowerCase() === program.toLowerCase());
      if (running) program = running.name;
    }
    if (!draft.programs.includes(program)) draft.programs = [...draft.programs, program];
    draft.entry = "";
    this.requestUpdate();
    return true;
  }

  removeAppRuleProgram(program) {
    if (!this.appRuleDraft) return;
    this.appRuleDraft.programs = this.appRuleDraft.programs.filter((item) => item !== program);
    this.requestUpdate();
  }

  /** Replaces a program path with its folder, which also matches helper programs there. */
  useProgramFolder(program) {
    const draft = this.appRuleDraft;
    if (!draft || !program.includes("/") || program.endsWith("/")) return;
    const folder = program.slice(0, program.lastIndexOf("/") + 1);
    draft.programs = [...new Set(draft.programs.map((item) => (item === program ? folder : item)))];
    this.requestUpdate();
  }

  async browseProgram() {
    const result = await this.preview("apps:pick", () => api.pickProgram());
    if (result?.path) this.addAppRuleProgram(result.path);
  }

  async loadRunningPrograms() {
    const result = await this.preview("apps:running", () => api.runningPrograms());
    if (result) {
      this.runningPrograms = result.programs || [];
      this.requestUpdate();
    }
  }

  async checkLocalProxyPort() {
    const port = Number(this.appRuleDraft?.localProxy?.port);
    if (!Number.isInteger(port) || port < 1 || port > 65535) return;
    const result = await this.preview("apps:port", () => api.checkLocalProxy(port));
    if (result) {
      this.portCheck = { port, ...result };
      this.requestUpdate();
    }
  }

  appRuleInput(draft) {
    const rule = {
      id: draft.id,
      name: draft.name.trim(),
      programs: draft.programs,
      target: draft.target,
      enabled: draft.enabled !== false,
    };
    if (draft.target === "local")
      rule.localProxy = {
        protocol: draft.localProxy.protocol,
        host: draft.localProxy.host.trim(),
        port: Number(draft.localProxy.port),
      };
    return rule;
  }

  async saveAppRule(rule = null) {
    const input = rule || (this.appRuleDraft && this.appRuleInput(this.appRuleDraft));
    if (!input || (!rule && Object.keys(appRuleErrors(this.appRuleDraft)).length)) return;
    const result = await this.run("apps:save", () => api.saveAppRule(input), [], `${this.t("appRules")} · ${input.name}`);
    if (result) {
      if (!rule) this.appRuleDraft = null;
      this.portCheck = null;
      await this.loadAppRules();
    }
  }

  removeAppRule(rule) {
    this.requestConfirmation(
      [rule.name, ...rule.programs],
      async () => {
        const result = await this.run(`apps:remove:${rule.id}`, () => api.removeAppRule(rule.id), [], `${this.t("remove")} · ${rule.name}`);
        if (result) await this.loadAppRules();
      },
      true,
      { title: this.t("removeAppRuleTitle"), confirmLabel: this.t("remove") },
    );
  }

  // ---- Hosts file ---------------------------------------------------------

  async loadHosts() {
    const result = await this.preview("hosts:load", () => api.hostsFile());
    if (result) {
      this.hosts = result;
      this.hostsEdit = (result.lines || []).map((line, index) => ({ ...line, key: index }));
      this.requestUpdate();
    }
  }

  hostsDirty() {
    if (!this.hosts || !this.hostsEdit) return false;
    const strip = (lines) => JSON.stringify(lines.map(({ key, ...line }) => line));
    return strip(this.hostsEdit) !== strip(this.hosts.lines || []);
  }

  updateHostsLine(key, change) {
    this.hostsEdit = this.hostsEdit.map((line) => (line.key === key ? { ...line, ...change } : line));
    this.requestUpdate();
  }

  addHostsLine(entry) {
    const key = Math.max(-1, ...this.hostsEdit.map((line) => line.key)) + 1;
    this.hostsEdit = [...this.hostsEdit, { kind: "entry", enabled: true, comment: "", ...entry, key }];
    this.requestUpdate();
  }

  removeHostsLine(key) {
    this.hostsEdit = this.hostsEdit.filter((line) => line.key !== key);
    this.requestUpdate();
  }

  async saveHosts() {
    if (!this.hostsDirty()) return;
    const input = { lines: this.hostsEdit.map(({ key, ...line }) => line), baseHash: this.hosts.hash };
    const preview = await this.preview("hosts:preview", () => api.previewHostsChange(input));
    if (!preview) return;
    const rows = [
      ...(preview.diff?.added || []).map((text) => ({ label: this.t("hostsAdded"), after: text, kind: "add" })),
      ...(preview.diff?.removed || []).map((text) => ({ label: this.t("hostsRemoved"), after: text, kind: "remove" })),
    ];
    this.requestConfirmation(
      this.warningsOf(preview),
      async () => {
        const result = await this.run("hosts", () => api.applyHostsChange(input, preview.confirmationToken), [], this.t("hosts"));
        if (result) await this.loadHosts();
      },
      true,
      { title: this.t("confirmHostsTitle"), confirmLabel: this.t("apply"), rows, acknowledge: this.t("hostsAcknowledge") },
    );
  }

  // ---- Monitoring (read-only) --------------------------------------------

  syncMonitorPolling() {
    const wanted = this.view === "monitor" && !globalThis.document?.hidden;
    if (wanted && !this.monitorTimer) {
      this.loadMonitorSnapshot();
      if (this.monitor.events == null) this.loadNetworkEvents();
      this.monitorTimer = setInterval(() => this.loadMonitorSnapshot(), MONITOR_POLL_MS);
    } else if (!wanted) {
      this.stopMonitorPolling();
    }
    this.syncHistoryPolling();
  }

  /** Saved history reloads once a minute while a history range is shown. */
  syncHistoryPolling() {
    const wanted = this.view === "monitor" && !globalThis.document?.hidden && this.monitor.range !== "live";
    if (wanted && !this.historyTimer) {
      this.loadMonitorHistory();
      this.historyTimer = setInterval(() => this.loadMonitorHistory(), HISTORY_POLL_MS);
    } else if (!wanted && this.historyTimer) {
      clearInterval(this.historyTimer);
      this.historyTimer = null;
    }
  }

  stopMonitorPolling() {
    clearInterval(this.monitorTimer);
    this.monitorTimer = null;
    clearInterval(this.historyTimer);
    this.historyTimer = null;
  }

  saveMonitorPrefs() {
    const { range, selected, cards, rateUnit } = this.monitor;
    writeStorage("wnm-monitor-prefs", JSON.stringify({ range, selected, cards, rateUnit }));
  }

  setMonitorRange(range) {
    if (!MONITOR_RANGES.includes(range) || range === this.monitor.range) return;
    this.monitor.range = range;
    this.monitor.history = null;
    this.monitor.hover = null;
    this.saveMonitorPrefs();
    clearInterval(this.historyTimer);
    this.historyTimer = null;
    this.syncHistoryPolling();
    this.requestUpdate();
  }

  toggleMonitorInterface(key) {
    const selected = new Set(this.monitor.selected);
    selected.has(key) ? selected.delete(key) : selected.add(key);
    this.monitor.selected = [...selected].slice(0, 16);
    this.saveMonitorPrefs();
    if (this.monitor.range !== "live") this.loadMonitorHistory();
    this.requestUpdate();
  }

  setMonitorCard(card, visible) {
    this.monitor.cards = { ...this.monitor.cards, [card]: visible };
    this.saveMonitorPrefs();
    this.requestUpdate();
  }

  setRateUnit(unit) {
    this.monitor.rateUnit = unit === "bytes" ? "bytes" : "bits";
    this.saveMonitorPrefs();
    this.requestUpdate();
  }

  async loadMonitorHistory() {
    const range = this.monitor.range;
    if (range === "live") return;
    try {
      const history = await api.monitorHistory({ range, interfaces: this.monitor.selected });
      if (this.monitor.range === range) this.monitor.history = history;
    } catch (error) {
      this.notify(errorMessage(error), "error");
    }
    this.requestUpdate();
  }

  // ---- Monitor history settings -------------------------------------------

  async loadMonitorStorage() {
    try {
      this.monitor.storage = await api.monitorStorage();
      this.requestUpdate();
    } catch {
      // Storage details are unavailable outside the Tauri runtime.
    }
  }

  async updateMonitorSettings(change) {
    const current = { ...defaultMonitorSettings, ...(this.settings.monitor || {}) };
    const apply = async () => {
      await this.updateSettings({ monitor: { ...current, ...change } });
      await this.loadMonitorStorage();
    };
    if (change.retentionDays && change.retentionDays < current.retentionDays) {
      this.requestConfirmation(
        [this.tf("retentionShorterHelp", { days: change.retentionDays })],
        apply,
        true,
        { title: this.t("retentionShorterTitle"), confirmLabel: this.t("apply") },
      );
      return;
    }
    await apply();
  }

  async chooseMonitorFolder() {
    const result = await this.preview("monitor:folder", () => api.chooseMonitorFolder());
    if (!result || result.cancelled) return;
    const candidate = result.candidate || {};
    this.requestConfirmation(
      [
        candidate.path,
        candidate.days ? this.tf("folderHasHistory", { days: candidate.days }) : "",
        this.t("folderCopyHelp"),
      ],
      async () => {
        const changed = await this.run("monitor:folder:set", () => api.setMonitorFolder(result.confirmationToken, true), [], this.t("changeFolder"));
        if (changed) this.applySettingsResult(changed);
      },
      false,
      { title: this.t("useThisFolderTitle"), confirmLabel: this.t("useThisFolder") },
    );
  }

  async resetMonitorFolder() {
    const changed = await this.run("monitor:folder:reset", () => api.resetMonitorFolder(), [], this.t("useDefaultFolder"));
    if (changed) this.applySettingsResult(changed);
  }

  applySettingsResult(result) {
    if (result.settings) this.settings = { ...this.settings, ...result.settings };
    if (result.storage) this.monitor.storage = result.storage;
    this.requestUpdate();
  }

  clearMonitorHistory() {
    const storage = this.monitor.storage;
    this.requestConfirmation(
      [storage?.path || "", this.tf("clearMonitorHistoryHelp", { days: storage?.days ?? 0 })],
      async () => {
        const result = await this.run("monitor:history:clear", () => api.clearMonitorHistory(), [], this.t("clearMonitorHistory"));
        if (result?.storage) this.monitor.storage = result.storage;
        this.monitor.history = null;
        this.loadMonitorHistory();
        this.requestUpdate();
      },
      true,
      { title: this.t("clearMonitorHistory"), confirmLabel: this.t("delete") },
    );
  }

  async openMonitorFolder() {
    await this.preview("monitor:folder:open", () => api.openMonitorFolder());
  }

  /** Bits or bytes per second, as chosen on the Monitor page. */
  formatRate(value) {
    return this.monitor.rateUnit === "bytes" ? formatByteRate(value) : formatRate(value);
  }

  async loadMonitorSnapshot() {
    try {
      this.monitor.snapshot = await api.monitorSnapshot();
      this.requestUpdate();
    } catch {
      // Sampling resumes on the next poll.
    }
  }

  async loadNetworkEvents() {
    const result = await this.preview("monitor:events", () => api.networkEvents());
    if (result) {
      this.monitor.events = result.events || [];
      this.requestUpdate();
    }
  }

  onNetworkEvents(events) {
    if (!Array.isArray(events) || this.monitor.events == null) return;
    this.monitor.events = [...[...events].reverse(), ...this.monitor.events].slice(0, 500);
    this.requestUpdate();
  }

  clearNetworkEvents() {
    this.requestConfirmation(
      [this.t("clearHistoryHelp")],
      async () => {
        const result = await this.run("monitor:clear", () => api.clearNetworkEvents(), [], this.t("clearHistory"));
        if (result) {
          this.monitor.events = [];
          this.requestUpdate();
        }
      },
      true,
      { title: this.t("clearHistory"), confirmLabel: this.t("delete") },
    );
  }

  setEventFilter(change) {
    this.monitor.eventFilters = { ...this.monitor.eventFilters, ...change };
    this.requestUpdate();
  }

  setMonitorHover(hover) {
    this.monitor.hover = hover;
    this.requestUpdate();
  }

  updateNotifications(change) {
    return this.updateSettings({ notifications: { ...this.settings.notifications, ...change } });
  }

  selectedDefaults() {
    if (!this.defaultsSelection)
      this.defaultsSelection = new Set(
        this.defaultsCandidates().map((item) => item.interfaceIndex),
      );
    return this.defaultsSelection;
  }

  async restoreDefaults() {
    const selected = this.selectedDefaults();
    const selection = {
      interfaceIndexes: this.defaultsCandidates()
        .map((item) => item.interfaceIndex)
        .filter((index) => selected.has(index)),
      removeManagedRoutes: this.defaultsRemoveRoutes,
    };
    const preview = await this.preview("defaults-preview", () => api.defaultsPreview(selection));
    if (!preview) return;
    const rows = [
      ...(preview.interfaces || []).map((item) => ({
        label: item.alias,
        after: this.t("automaticMetric"),
      })),
      ...(preview.managedRoutes || []).map((route) => ({
        label: this.t("changeRouteRemove"),
        after: `${route.destinationPrefix} → ${route.nextHop}`,
        kind: "remove",
      })),
    ];
    this.requestConfirmation(
      preview.notices?.length
        ? this.warningsOf(preview)
        : [preview.metrics, preview.routes, preview.wifi],
      () =>
        this.run(
          "defaults",
          () => api.defaults(selection, preview.confirmationToken),
          ["interfaces", "routes"],
          this.t("restoreDefaults"),
        ),
      true,
      { title: this.t("restoreDefaults"), confirmLabel: this.t("restore"), rows },
    );
  }

  // ---- Rendering ----------------------------------------------------------

  viewSubtitle() {
    const adapters = this.interfaces();
    const online = adapters.filter(isAdapterOnline).length;
    switch (this.view) {
      case "overview":
      case "adapters":
        return this.tf("subtitleAdapters", { total: adapters.length, online });
      case "routes":
        return this.tf("routeCount", {
          shown: this.routeRows().length,
          total: (this.network.routes?.routes || []).length,
        });
      case "profiles":
        return this.tf("subtitleBackups", {
          profiles: this.config.profiles?.length || 0,
          recent: this.config.recentSnapshots?.length || 0,
        });
      default:
        return this.t(`${this.view}Subtitle`);
    }
  }

  renderNav() {
    const activeRules = this.activeAutoRuleCount();
    const navButton = (view) =>
      html`<button
        class="nav-item ${this.view === view ? "selected" : ""}"
        aria-current=${this.view === view ? "page" : "false"}
        title=${views.indexOf(view) < 9 ? `${this.t(view)} (Ctrl+${views.indexOf(view) + 1})` : this.t(view)}
        @click=${() => this.setView(view)}
      >
        ${icon(view)}<span class="nav-label">${this.t(view)}</span>
      </button>`;
    return html`<aside class="sidebar">
      <div class="brand">
        <span class="brand-mark">${icon("globe")}</span>
        <div class="nav-label">
          <strong>${this.t("appName")}</strong>
          <small>${this.version ? `v${this.version}` : ""}</small>
        </div>
      </div>
      <nav aria-label=${this.t("primaryNavigation")}>
        ${views.filter((view) => view !== "settings").map(navButton)}
      </nav>
      <div class="sidebar-footer">
        ${this.privilege.isAdmin
          ? html`<div class="side-status admin" title=${this.t("administrator")}>
              ${icon("shield")}
              <div class="nav-label">
                <strong>${this.t("administrator")}</strong>
                <small>${this.t("adminReady")}</small>
              </div>
            </div>`
          : html`<button
              class="side-status"
              title=${`${this.t("permissionHelp")} ${this.t("restartAdmin")}`}
              ?disabled=${this.isBusy("elevation")}
              @click=${() => this.restartAsAdmin()}
            >
              ${this.isBusy("elevation") ? spinner() : icon("user")}
              <div class="nav-label">
                <strong>${this.t("standardUser")}</strong>
                <small class="side-action">${this.t("restartAdmin")}</small>
              </div>
            </button>`}
        <button
          class="side-status side-auto"
          aria-describedby="side-auto-tip"
          @click=${() => this.goToAdapters("wifi")}
        >
          ${icon("wifi")}
          <div class="nav-label">
            <strong>${this.t("wifiAutoConnect")}</strong>
            <small
              >${activeRules
                ? this.tf("autoActiveCount", { count: activeRules })
                : this.t("autoNone")}</small
            >
          </div>
          <div class="side-tip" id="side-auto-tip" role="tooltip">
            <strong>${this.t("wifiAutoConnect")}</strong>
            <p>${this.t("autoTipWhat")}</p>
            <p>${this.t("autoTipHow")}</p>
            <p class="side-tip-note">${this.t("autoTipTray")}</p>
          </div>
        </button>
        ${navButton("settings")}
      </div>
    </aside>`;
  }

  renderHeader() {
    const refreshing = this.isRefreshing();
    const sourceLabel = this.network.stale ? this.t("cached") : this.t("live");
    const running = this.activity.filter((entry) => entry.status === "running").length;
    const attention = this.activity.filter((entry) => ["failed", "partial"].includes(entry.status) && !entry.seen).length;
    return html`<header class="app-header">
      <div class="title-block">
        <h1>${this.t(this.view)}</h1>
        <p>${this.viewSubtitle()}</p>
      </div>
      <div class="header-actions">
        <span
          class="freshness ${this.network.stale ? "stale" : ""} ${refreshing ? "refreshing" : ""}"
          title=${`${sourceLabel} · ${this.network.source} · ${formatTime(this.network.refreshedAt, this.settings.language)}`}
          ><span class="pulse-dot"></span>${refreshing
            ? this.t("refreshing")
            : `${sourceLabel} · ${formatRelative(this.network.refreshedAt, this.settings.language)}`}</span
        >
        <button
          class="icon-button standard"
          title=${`${this.t("refresh")} (F5)`}
          aria-label=${this.t("refresh")}
          ?disabled=${refreshing}
          @click=${() => this.refresh(["interfaces", "routes"], true)}
        >
          ${refreshing ? spinner() : icon("refresh")}
        </button>
        <button
          class="icon-button standard activity-button"
          title=${this.t("activity")}
          aria-label=${this.t("activity")}
          @click=${() => {
            this.activityOpen = true;
            this.activity.forEach((entry) => (entry.seen = true));
            this.requestUpdate();
          }}
        >
          ${running ? spinner() : icon("activity")}${attention
            ? html`<span class="badge">${attention}</span>`
            : nothing}
        </button>
      </div>
    </header>`;
  }

  renderToasts() {
    const icons = { success: "success", error: "error", warning: "warning", info: "info" };
    return html`<div class="toast-host" role="status" aria-live="polite">
      ${this.toasts.map(
        (toast) =>
          html`<div class="toast ${toast.kind}">
            ${icon(icons[toast.kind] || "info")}
            <span>${toast.text}</span>
            <button
              class="icon-button subtle"
              aria-label=${this.t("close")}
              @click=${() => this.dismissToast(toast.id)}
            >
              ${icon("close")}
            </button>
          </div>`,
      )}
    </div>`;
  }

  renderActivityResults(result) {
    const steps = Array.isArray(result?.results) ? result.results : [];
    if (!steps.length) return nothing;
    const status = (step) => String(step.status || "").toLowerCase();
    return html`<ul class="step-list">
      ${steps.map(
        (step) =>
          html`<li class=${status(step)}>
            ${icon(
              status(step) === "failed" ? "error" : status(step) === "skipped" ? "info" : "success",
            )}<span
              >${step.kind ? html`<strong>${step.kind}</strong> ` : nothing}${step.message ||
              step.error ||
              step.alias ||
              step.destinationPrefix ||
              step.status ||
              ""}</span
            >
          </li>`,
      )}
    </ul>`;
  }

  renderActivity() {
    const statusIcon = { success: "success", partial: "warning", failed: "error" };
    return html`<header class="drawer-header">
        <h2>${this.t("activity")}</h2>
        <button
          class="icon-button subtle"
          aria-label=${this.t("close")}
          @click=${() => {
            this.activityOpen = false;
            this.requestUpdate();
          }}
        >
          ${icon("close")}
        </button>
      </header>
      <div class="drawer-body">
        <p class="help-text">${this.t("activityHelp")}</p>
        ${this.activity.length
          ? html`<ol class="activity-list">
              ${this.activity.map(
                (entry) =>
                  html`<li class="activity-entry ${entry.status}">
                    <span class="activity-icon"
                      >${entry.status === "running" ? spinner() : icon(statusIcon[entry.status])}</span
                    >
                    <div>
                      <strong>${entry.label}</strong>
                      <small>${formatTime(entry.time, this.settings.language)}</small>
                      ${entry.message ? html`<p>${entry.message}</p>` : nothing}
                      ${this.renderActivityResults(entry.result)}
                      ${entry.result
                        ? html`<details>
                            <summary>${this.t("technicalDetails")}</summary>
                            <pre>${JSON.stringify(entry.result, null, 2)}</pre>
                          </details>`
                        : nothing}
                    </div>
                  </li>`,
              )}
            </ol>`
          : html`<p class="empty">${this.t("activityEmpty")}</p>`}
      </div>`;
  }

  renderConfirmation() {
    const confirmation = this.confirmation;
    return html`<form
      class="dialog-body"
      @submit=${(event) => {
        event.preventDefault();
        this.confirm();
      }}
    >
      <h2 id="dialog-heading" class="dialog-title ${confirmation.destructive ? "is-danger" : ""}">
        ${icon(confirmation.destructive ? "warning" : "info")}${confirmation.title}
      </h2>
      ${confirmation.rows.length
        ? html`<div class="change-rows">
            ${confirmation.rows.map(
              (row) =>
                html`<div class="change-row ${row.kind || ""}">
                  <small>${row.label}</small>
                  <span
                    >${row.before != null
                      ? html`<span class="before">${row.before}</span>${icon("arrowRight")}`
                      : nothing}<span class="after">${row.after}</span></span
                  >
                </div>`,
            )}
          </div>`
        : nothing}
      <ul class="confirm-lines">
        ${confirmation.lines.map((line) => html`<li>${line}</li>`)}
      </ul>
      ${confirmation.note ? html`<p class="help-text">${confirmation.note}</p>` : nothing}
      ${confirmation.acknowledge
        ? html`<label class="check acknowledge"
            ><input
              type="checkbox"
              .checked=${confirmation.acknowledged}
              @change=${(event) => {
                confirmation.acknowledged = event.target.checked;
                this.requestUpdate();
              }}
            /><span>${confirmation.acknowledge}</span></label
          >`
        : nothing}
      ${confirmation.typeToConfirm
        ? html`<label class="field"
            >${this.tf("typeToConfirm", { value: confirmation.typeToConfirm })}<input
              autocomplete="off"
              spellcheck="false"
              .value=${confirmation.typed}
              @input=${(event) => {
                confirmation.typed = event.target.value;
                this.requestUpdate();
              }}
          /></label>`
        : nothing}
      <div class="dialog-actions">
        <button
          type="button"
          class="standard"
          ?data-autofocus=${confirmation.destructive}
          @click=${() => this.closeConfirmation()}
        >
          ${this.t("cancel")}
        </button>
        <button
          type="submit"
          class=${confirmation.destructive ? "danger-fill" : "accent"}
          ?data-autofocus=${!confirmation.destructive}
          ?disabled=${!this.confirmationReady()}
        >
          ${confirmation.confirmLabel}
        </button>
      </div>
    </form>`;
  }

  renderNameDialog() {
    const kind = this.nameDialog.kind;
    const backup = kind === "backup" || kind === "renameBackup";
    const text = {
      snapshot: ["save", "saveSnapshot", "snapshotHelp", "snapshotName"],
      profile: ["profiles", "saveProfile", "profileHelp", "configurationName"],
      backup: ["save", "createBackup", "createBackupHelp", "backupName"],
      renameBackup: ["edit", "renameBackup", "renameBackupHelp", "backupName"],
    }[kind] || ["profiles", "saveProfile", "profileHelp", "configurationName"];
    const [iconName, title, help, label] = text;
    return html`<form class="dialog-body" @submit=${this.saveNamed}>
      <h2 id="dialog-heading" class="dialog-title">${icon(iconName)}${this.t(title)}</h2>
      <p class="help-text">${this.t(help)}</p>
      <label class="field"
        >${this.t(label)}<input
          name="name"
          ?required=${!backup}
          maxlength="80"
          .value=${this.nameDialog.value || ""}
          placeholder=${backup ? this.t("backupNamePlaceholder") : ""}
          data-autofocus
      /></label>
      <div class="dialog-actions">
        <button type="button" class="standard" @click=${() => this.closeNameDialog()}>
          ${this.t("cancel")}
        </button>
        <button type="submit" class="accent">${this.t(kind === "backup" ? "createBackup" : "save")}</button>
      </div>
    </form>`;
  }

  renderNewWifiDialog() {
    const dialog = this.newWifi;
    const open = dialog.network.security === "Open";
    return html`<form
      class="dialog-body"
      autocomplete="off"
      @submit=${(event) => {
        event.preventDefault();
        this.submitNewWifi();
      }}
    >
      <h2 id="dialog-heading" class="dialog-title">${icon("wifi")}${this.t("connectNewWifi")}</h2>
      <p><strong>${dialog.network.ssid}</strong> · ${dialog.network.security} · ${dialog.preview.interfaceName}</p>
      <ul class="confirm-lines">
        ${this.warningsOf(dialog.preview).map((line) => html`<li>${line}</li>`)}
      </ul>
      ${open
        ? nothing
        : html`<label class="field"
              >${this.t("wifiPassword")}<input
                type=${dialog.show ? "text" : "password"}
                autocomplete="new-password"
                spellcheck="false"
                maxlength="64"
                data-autofocus
                .value=${dialog.password}
                @input=${(event) => {
                  dialog.password = event.target.value;
                  this.requestUpdate();
                }}
            /></label>
            <label class="check"
              ><input
                type="checkbox"
                .checked=${dialog.show}
                @change=${(event) => {
                  dialog.show = event.target.checked;
                  this.requestUpdate();
                }}
              /><span>${this.t("showPassword")}</span></label
            >`}
      <div class="dialog-actions">
        <button type="button" class="standard" @click=${() => this.closeNewWifi()}>${this.t("cancel")}</button>
        <button type="submit" class="accent" ?disabled=${!open && dialog.password.length < 8}>${this.t("connect")}</button>
      </div>
    </form>`;
  }

  renderRevertDialog() {
    const seconds = this.revertSecondsLeft();
    return html`<div class="dialog-body">
      <h2 id="dialog-heading" class="dialog-title">${icon("clock")}${this.t("keepSettingsTitle")}</h2>
      <p>${this.pendingRevert.label}</p>
      <p class="help-text">${this.tf("keepSettingsHelp", { seconds })}</p>
      <div class="revert-countdown" role="timer" aria-live="polite">${seconds}</div>
      <div class="dialog-actions">
        <button type="button" class="standard" ?disabled=${this.isBusy("revert:now")} @click=${() => this.revertChange()}>
          ${this.t("revertNow")}
        </button>
        <button type="button" class="accent" data-autofocus ?disabled=${this.isBusy("revert:keep")} @click=${() => this.keepChange()}>
          ${this.t("keepSettings")}
        </button>
      </div>
    </div>`;
  }

  renderDialog() {
    const content = this.confirmation
      ? this.renderConfirmation()
      : this.nameDialog
        ? this.renderNameDialog()
        : this.pendingRevert
          ? this.renderRevertDialog()
          : this.newWifi
            ? this.renderNewWifiDialog()
            : nothing;
    return html`<dialog
      id="app-dialog"
      class="modal"
      aria-labelledby="dialog-heading"
      @cancel=${(event) => {
        event.preventDefault();
        // Escape on "Keep these settings?" means revert, the safe choice.
        if (!this.confirmation && !this.nameDialog && this.pendingRevert) return this.revertChange();
        this.closeConfirmation();
        this.closeNameDialog();
        this.closeNewWifi();
      }}
      @click=${(event) => {
        if (event.target === event.currentTarget && !(this.pendingRevert && !this.confirmation)) {
          this.closeConfirmation();
          this.closeNameDialog();
        }
      }}
    >
      ${content}
    </dialog>`;
  }

  renderDrawers() {
    const close = (event, handler) => {
      event.preventDefault();
      handler();
    };
    return html`<dialog
        id="route-drawer"
        class="drawer"
        aria-labelledby="route-drawer-heading"
        @cancel=${(event) => close(event, () => this.closeRouteDrawer())}
        @click=${(event) => {
          if (event.target === event.currentTarget) this.closeRouteDrawer();
        }}
      >
        ${this.routeDrawerOpen ? renderRouteDrawer(this) : nothing}
      </dialog>
      <dialog
        id="ip-drawer"
        class="drawer"
        aria-labelledby="ip-drawer-heading"
        @cancel=${(event) => close(event, () => this.closeIpEditor())}
        @click=${(event) => {
          if (event.target === event.currentTarget) this.closeIpEditor();
        }}
      >
        ${this.ipEditor ? renderIpDrawer(this) : nothing}
      </dialog>
      <dialog
        id="activity-drawer"
        class="drawer"
        aria-label=${this.t("activity")}
        @cancel=${(event) =>
          close(event, () => {
            this.activityOpen = false;
            this.requestUpdate();
          })}
        @click=${(event) => {
          if (event.target === event.currentTarget) {
            this.activityOpen = false;
            this.requestUpdate();
          }
        }}
      >
        ${this.activityOpen ? this.renderActivity() : nothing}
      </dialog>`;
  }

  syncDialog(selector, open) {
    const dialog = this.querySelector(selector);
    if (!dialog?.showModal) return;
    if (open && !dialog.open) {
      dialog.showModal();
      (dialog.querySelector("[data-autofocus]") || dialog.querySelector("input, select, button"))?.focus();
    } else if (!open && dialog.open) {
      dialog.close();
    }
  }

  updated() {
    // Modal dialogs are stacked: the confirmation can open above the route drawer.
    this.syncDialog("#route-drawer", this.routeDrawerOpen);
    this.syncDialog("#activity-drawer", this.activityOpen);
    this.syncDialog("#ip-drawer", Boolean(this.ipEditor));
    this.syncDialog("#app-dialog", Boolean(this.confirmation || this.nameDialog || this.pendingRevert || this.newWifi));
  }

  render() {
    return html`<div class="app-shell">
      ${this.renderNav()}
      <main>
        ${this.renderHeader()}
        <div class="view" data-view=${this.view}>
          ${viewRenderers[this.view](this)}
        </div>
      </main>
      ${this.renderToasts()} ${this.renderDrawers()} ${this.renderDialog()}
    </div>`;
  }
}

customElements.define("network-manager-app", NetworkManagerApp);
