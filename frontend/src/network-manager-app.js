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
import { spinner } from "./ui.js";
import { renderOverview } from "./views/overview.js";
import { renderAdapters } from "./views/adapters.js";
import { renderRoutes, renderRouteDrawer } from "./views/routes.js";
import { renderProfiles } from "./views/profiles.js";
import { renderDiagnostics } from "./views/diagnostics.js";
import { renderMaintenance } from "./views/maintenance.js";
import { renderSettings } from "./views/settings.js";

export const views = [
  "overview",
  "adapters",
  "routes",
  "profiles",
  "diagnostics",
  "maintenance",
  "settings",
];
const viewRenderers = {
  overview: renderOverview,
  adapters: renderAdapters,
  routes: renderRoutes,
  profiles: renderProfiles,
  diagnostics: renderDiagnostics,
  maintenance: renderMaintenance,
  settings: renderSettings,
};
const TOAST_TIMEOUT = { success: 4000, info: 5000 };

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
      showSystemInterfaces: false,
      showAllRoutes: false,
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
    this.diagnostics = null;
    this.refreshTimer = null;
    this.clockTimer = null;
    this.onVisibilityChange = () => this.scheduleRefresh();
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
    if (this.wifiAutoEventUnlisten) {
      this.wifiAutoEventUnlisten();
      this.wifiAutoEventUnlisten = null;
    }
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
      this.loadVersion();
      const bootstrap = await api.bootstrap();
      this.settings = { ...this.settings, ...(bootstrap.settings || {}) };
      this.config = { ...this.config, ...(bootstrap.config || bootstrap) };
      this.privilege = bootstrap.privilege || this.privilege;
      if (bootstrap.cachedNetworkState)
        this.applyNetwork(bootstrap.cachedNetworkState);
      this.applyTheme();
      this.requestUpdate();
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
    };
    this.requestUpdate();
  }

  closeConfirmation() {
    this.confirmation = null;
    this.requestUpdate();
  }
  async confirm() {
    const action = this.confirmation?.confirm;
    this.closeConfirmation();
    if (action) await action();
  }

  // ---- Navigation and shortcuts ------------------------------------------

  setView(view) {
    if (!views.includes(view)) return;
    this.view = view;
    writeStorage("wnm-view", view);
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
      this.confirmation || this.nameDialog || this.routeDrawerOpen || this.activityOpen,
    );
  }

  handleShortcut(event) {
    if (event.key === "F5" || (event.ctrlKey && event.key.toLowerCase() === "r")) {
      event.preventDefault();
      this.refresh(["interfaces", "routes"], true);
      return;
    }
    if (this.anyDialogOpen()) return;
    if (event.ctrlKey && !event.altKey && /^[1-7]$/.test(event.key)) {
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
        [`${item.alias} (#${item.interfaceIndex})`, ...this.warningsOf(preview)],
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
          title: this.t(action === "enable" ? "confirmEnableTitle" : "confirmDisableTitle"),
          confirmLabel: this.t(action),
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
    };
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

  openNameDialog(kind) {
    this.nameDialog = { kind, value: "" };
    this.requestUpdate();
  }
  closeNameDialog() {
    this.nameDialog = null;
    this.requestUpdate();
  }
  async saveNamed(event) {
    event.preventDefault();
    const name = new FormData(event.currentTarget).get("name")?.trim();
    if (!name) return;
    const kind = this.nameDialog.kind;
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
    return changes.map((change) => {
      switch (change.kind) {
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
    return this.interfaces().filter((item) => !item.systemInterface);
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
        title=${`${this.t(view)} (Ctrl+${views.indexOf(view) + 1})`}
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
        <div
          class="side-status"
          title=${activeRules ? this.tf("autoActiveCount", { count: activeRules }) : this.t("autoNone")}
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
        </div>
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
        >
          ${confirmation.confirmLabel}
        </button>
      </div>
    </form>`;
  }

  renderNameDialog() {
    const snapshot = this.nameDialog.kind === "snapshot";
    return html`<form class="dialog-body" @submit=${this.saveNamed}>
      <h2 id="dialog-heading" class="dialog-title">
        ${icon(snapshot ? "save" : "profiles")}${snapshot ? this.t("saveSnapshot") : this.t("saveProfile")}
      </h2>
      <p class="help-text">${snapshot ? this.t("snapshotHelp") : this.t("profileHelp")}</p>
      <label class="field"
        >${snapshot ? this.t("snapshotName") : this.t("configurationName")}<input
          name="name"
          required
          maxlength="80"
          data-autofocus
      /></label>
      <div class="dialog-actions">
        <button type="button" class="standard" @click=${() => this.closeNameDialog()}>
          ${this.t("cancel")}
        </button>
        <button type="submit" class="accent">${this.t("save")}</button>
      </div>
    </form>`;
  }

  renderDialog() {
    const content = this.confirmation
      ? this.renderConfirmation()
      : this.nameDialog
        ? this.renderNameDialog()
        : nothing;
    return html`<dialog
      id="app-dialog"
      class="modal"
      aria-labelledby="dialog-heading"
      @cancel=${(event) => {
        event.preventDefault();
        this.closeConfirmation();
        this.closeNameDialog();
      }}
      @click=${(event) => {
        if (event.target === event.currentTarget) {
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
    this.syncDialog("#app-dialog", Boolean(this.confirmation || this.nameDialog));
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
