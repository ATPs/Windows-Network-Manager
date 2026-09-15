import { LitElement, html, nothing } from "lit";
import { api } from "./api.js";
import { copy } from "./i18n.js";

const views = [
  "overview",
  "adapters",
  "routes",
  "profiles",
  "diagnostics",
  "maintenance",
];
const formatTime = (value, language) =>
  value
    ? new Intl.DateTimeFormat(language, {
        dateStyle: "medium",
        timeStyle: "short",
      }).format(new Date(value))
    : "—";
const errorMessage = (error) => error?.message || error?.code || String(error);

export class NetworkManagerApp extends LitElement {
  createRenderRoot() {
    return this;
  }

  constructor() {
    super();
    this.view = "overview";
    this.settings = {
      language: "en",
      theme: localStorage.getItem("wnm-theme") || "system",
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
    this.config = { profiles: [], recentSnapshots: [], hasBaseline: false };
    this.privilege = { isAdmin: false };
    this.busy = new Set();
    this.wifiProfiles = new Map();
    this.message = null;
    this.confirmation = null;
    this.nameDialog = null;
    this.routeSearch = "";
    this.routeFamily = "IPv4";
    this.diagnostics = null;
    this.operationResult = null;
    this.adapterSearch = "";
    this.refreshTimer = null;
    this.onVisibilityChange = () => this.scheduleRefresh();
    this.onKeydown = (event) => {
      if (event.key === "Escape" && (this.confirmation || this.nameDialog)) {
        this.closeConfirmation();
        this.closeNameDialog();
      }
    };
  }

  connectedCallback() {
    super.connectedCallback();
    document.addEventListener("visibilitychange", this.onVisibilityChange);
    document.addEventListener("keydown", this.onKeydown);
    this.initialize();
  }
  disconnectedCallback() {
    super.disconnectedCallback();
    document.removeEventListener("visibilitychange", this.onVisibilityChange);
    document.removeEventListener("keydown", this.onKeydown);
    clearTimeout(this.refreshTimer);
  }

  t(key) {
    return copy[this.settings.language]?.[key] || copy.en[key] || key;
  }
  isBusy(key) {
    return this.busy.has(key);
  }
  setBusy(key, value) {
    value ? this.busy.add(key) : this.busy.delete(key);
    this.requestUpdate();
  }

  async initialize() {
    try {
      const bootstrap = await api.bootstrap();
      this.settings = { ...this.settings, ...(bootstrap.settings || {}) };
      this.config = { ...this.config, ...(bootstrap.config || bootstrap) };
      this.privilege = bootstrap.privilege || this.privilege;
      if (bootstrap.cachedNetworkState)
        this.applyNetwork(bootstrap.cachedNetworkState);
      this.applyTheme();
      this.requestUpdate();
      await this.refresh(["interfaces", "routes"]);
      this.scheduleRefresh();
    } catch (error) {
      this.notify(errorMessage(error), "error");
    }
  }

  applyTheme() {
    document.documentElement.lang = this.settings.language;
    document.documentElement.dataset.theme = this.settings.theme;
    localStorage.setItem("wnm-theme", this.settings.theme);
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

  async refresh(scopes = ["interfaces", "routes"], force = false, quiet = false) {
    const key = `refresh:${scopes.sort().join(",")}`;
    if (this.isBusy(key)) return;
    this.setBusy(key, true);
    try {
      this.applyNetwork(await api.refresh(scopes, force));
      if (!quiet && !this.operationResult)
        this.message = { text: this.t("ready"), kind: "success" };
      this.loadWifiProfiles();
    } catch (error) {
      this.notify(errorMessage(error), "error");
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
    await this.run("elevation", () => api.restartAsAdmin(), []);
  }

  notify(text, kind = "info") {
    this.message = { text, kind };
    this.requestUpdate();
  }
  dismissMessage() {
    this.message = null;
    this.requestUpdate();
  }

  async run(key, task, scopes = ["interfaces", "routes"]) {
    if (this.isBusy(key)) return;
    this.setBusy(key, true);
    try {
      const result = await task();
      const failed = JSON.stringify(result || {}).includes('"status":"failed"') || result?.partial;
      this.operationResult = result || { message: this.t("operationComplete") };
      this.notify(
        result?.message || result?.summary || (failed ? this.t("operationPartial") : this.t("operationComplete")),
        failed ? "warning" : "success",
      );
      if (scopes.length) await this.refresh(scopes, true, true);
      await this.refreshConfig().catch(() => {});
      return result;
    } catch (error) {
      this.notify(errorMessage(error), "error");
      return null;
    } finally {
      this.setBusy(key, false);
    }
  }

  requestConfirmation(lines, confirm, destructive = true) {
    this.confirmation = { lines: lines.filter(Boolean), confirm, destructive };
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

  interfaces() {
    const all = this.network.overview?.interfaces || [];
    return all.filter(
      (item) => this.settings.showSystemInterfaces || !item.systemInterface,
    );
  }

  routeRows() {
    const routes = this.network.routes?.routes || [];
    return routes.filter(
      (route) =>
        (this.routeFamily === "all" ||
          route.addressFamily === this.routeFamily) &&
        (this.settings.showAllRoutes || !route.hiddenByDefault) &&
        `${route.destinationPrefix} ${route.nextHop} ${route.interfaceAlias}`
          .toLowerCase()
          .includes(this.routeSearch.toLowerCase()),
    );
  }

  connectionLabel(item) {
    if (String(item.adminStatus).toLowerCase() === "down")
      return this.t("disabled");
    return item.wifi?.connected || item.defaultRoute
      ? this.t("connected")
      : this.t("available");
  }

  renderNav() {
    return html`<aside class="sidebar">
      <div class="brand">
        <span class="brand-mark">N</span>
        <div><strong>Windows Network Manager</strong><small>v2.1.1</small></div>
      </div>
      <nav aria-label="Primary navigation">
        ${views.map(
          (view) =>
            html`<button
              class="nav-item ${this.view === view ? "selected" : ""}"
              @click=${() => {
              this.view = view;
              this.requestUpdate();
            }}
            >
              ${this.t(view)}
            </button>`,
        )}
      </nav>
      <div class="sidebar-footer">
        <label
          >${this.t("theme")}<select
            .value=${this.settings.theme}
            @change=${(event) => this.updateSettings({ theme: event.target.value })}
          >
            <option value="system">${this.t("system")}</option>
            <option value="light">${this.t("light")}</option>
            <option value="dark">${this.t("dark")}</option>
          </select></label
        >
        <label
          >${this.t("language")}<select
            .value=${this.settings.language}
            @change=${(event) => this.updateSettings({ language: event.target.value })}
          >
            <option value="en">English</option>
            <option value="zh-CN">中文</option>
          </select></label
        >
      </div>
    </aside>`;
  }

  renderHeader() {
    const refreshKey = "refresh:interfaces,routes";
    return html`<header class="app-header">
      <div>
        <p class="eyebrow">${this.t(this.view)}</p>
        <h1>${this.t(this.view)}</h1>
      </div>
      <div class="header-actions">
        <span
          class="status-pill ${this.privilege.isAdmin ? "success" : "neutral"}"
          >${this.privilege.isAdmin ? this.t("administrator") : this.t("standardUser")}</span
        >
        ${!this.privilege.isAdmin ? html`<button class="secondary" ?disabled=${this.isBusy("elevation")} @click=${() => this.restartAsAdmin()}>${this.isBusy("elevation") ? this.t("working") : this.t("restartAdmin")}</button>` : nothing}
        <button
          class="primary"
          ?disabled=${this.isBusy(refreshKey)}
          @click=${() => this.refresh(["interfaces", "routes"])}
        >
          ${this.isBusy(refreshKey) ? this.t("refreshing") : this.t("refresh")}
        </button>
        <details class="settings-menu"><summary>${this.t("settings")}</summary><label>${this.t("theme")}<select .value=${this.settings.theme} @change=${(event) => this.updateSettings({ theme: event.target.value })}><option value="system">${this.t("system")}</option><option value="light">${this.t("light")}</option><option value="dark">${this.t("dark")}</option></select></label><label>${this.t("language")}<select .value=${this.settings.language} @change=${(event) => this.updateSettings({ language: event.target.value })}><option value="en">English</option><option value="zh-CN">中文</option></select></label></details>
      </div>
    </header>`;
  }

  renderOverview() {
    const adapters = this.interfaces();
    const active = adapters.find((item) => item.defaultRoute && String(item.adminStatus).toLowerCase() !== "down");
    const routes = this.network.routes?.routes || [];
    const manual = adapters.filter(
      (item) =>
        item.deviations?.manualIpv4Metric || item.deviations?.manualIpv6Metric,
    ).length;
    const disabled = adapters.filter(
      (item) => item.deviations?.adapterDisabled,
    ).length;
    const staticRoutes = routes.filter(
      (route) => route.persistent || route.protocol === "NetMgmt",
    ).length;
    return html`<section class="view-stack">
      <article class="hero-card">
        <div>
          <p class="eyebrow">${this.t("activePath")}</p>
          <h2>${active ? active.alias : this.t("noActivePath")}</h2>
          <p>${active?.wifi?.ssid || active?.description || this.t("noActivePath")}</p>
        </div>
        <div class="hero-meta">
          <span class="status-pill ${active ? "success" : "neutral"}"
            >${active ? this.connectionLabel(active) : "—"}</span
          ><small
            >${this.network.stale ? this.t("cached") : this.t("live")} ·
            ${formatTime(this.network.refreshedAt, this.settings.language)}</small
          >
        </div>
      </article>
      <section class="panel">
        <div class="section-heading">
          <div>
            <p class="eyebrow">${this.t("changes")}</p>
            <h2>${this.t("changes")}</h2>
          </div>
        </div>
        <div class="summary-grid">
          ${[
            [this.t("manualMetrics"), manual],
            [this.t("staticRoutes"), staticRoutes],
            [this.t("disabledAdapters"), disabled],
          ].map(
            ([label, value]) =>
              html`<div class="summary-tile">
                <strong>${value}</strong><span>${label}</span>
              </div>`,
          )}
        </div>
      </section>
      <section class="panel">
        <div class="section-heading">
          <div>
            <p class="eyebrow">${this.t("adapters")}</p>
            <h2>${this.t("adapters")}</h2>
          </div>
          <button
            class="secondary"
            @click=${() => {
              this.view = "adapters";
              this.requestUpdate();
            }}
          >
            ${this.t("adapters")}
          </button>
        </div>
        <div class="adapter-preview">
          ${adapters.slice(0, 4).map((item) => html`<div><span class="status-dot ${item.defaultRoute || item.wifi?.connected ? "online" : ""}"></span><strong>${item.alias}</strong><span>${item.type} · ${this.connectionLabel(item)}</span></div>`)}
        </div>
      </section>
      ${this.operationResult ? html`<section class="panel operation-result" aria-live="polite"><div class="section-heading"><div><p class="eyebrow">${this.t("lastOperation")}</p><h2>${this.operationResult.message || this.t("operationComplete")}</h2></div><button class="secondary compact" @click=${() => { this.operationResult = null; this.requestUpdate(); }}>${this.t("close")}</button></div><details><summary>${this.t("technicalDetails")}</summary><pre>${JSON.stringify(this.operationResult, null, 2)}</pre></details></section>` : nothing}
    </section>`;
  }

  renderMetric(item, family, key) {
    const metric = item.metrics?.[key];
    if (!metric) return nothing;
    const id = `metric-${item.interfaceIndex}-${key}`;
    const busyKey = `metric:${item.interfaceIndex}:${family}`;
    return html`<div class="metric-control">
      <strong>${family}</strong
      ><label
        ><input
          id=${`${id}-auto`}
          type="checkbox"
          .checked=${Boolean(metric.automaticMetric)}
          @change=${(event) => {
            const input = this.querySelector(`#${id}-value`);
            input.disabled = event.target.checked;
          }}
        />
        ${this.t("automaticMetric")}</label
      ><input
        id=${`${id}-value`}
        type="number"
        min="1"
        max="9999"
        .value=${String(metric.interfaceMetric)}
        ?disabled=${Boolean(metric.automaticMetric)}
      /><button
        class="secondary"
        ?disabled=${!this.privilege.isAdmin || this.isBusy(busyKey)}
        @click=${async () => {
          const automatic = this.querySelector(`#${id}-auto`).checked;
          const value = this.querySelector(`#${id}-value`).value;
          const input = {
            interfaceIndex: item.interfaceIndex,
            addressFamily: family,
            automatic,
            metric: value,
          };
          const preview = await this.run(
            busyKey,
            () => api.previewMetric(input),
            [],
          );
          if (preview?.unchanged)
            return this.notify(this.t("operationComplete"));
          if (preview)
            this.requestConfirmation(
              [`${item.alias} · ${family}`, ...(preview.warnings || [])],
              () =>
                this.run(busyKey, () =>
                  api.applyMetric(preview.input, preview.confirmationToken),
                ),
            );
        }}
      >
        ${this.isBusy(busyKey) ? this.t("working") : this.t("applyMetric")}
      </button>
    </div>`;
  }

  renderAdapter(item) {
    const wifiProfiles = this.wifiProfiles.get(item.alias);
    const adapterBusy = `adapter:${item.interfaceIndex}`;
    const disabled = String(item.adminStatus).toLowerCase() === "down";
    return html`<article class="adapter-card">
      <div class="adapter-top">
        <div>
          <div class="title-line">
            <span
              class="status-dot ${item.defaultRoute || item.wifi?.connected ? "online" : ""}"
            ></span>
            <h3>${item.alias}</h3>
            <span class="chip">${item.type}</span>
          </div>
          <p>${item.description || "—"}</p>
        </div>
        <span
          class="status-pill ${disabled ? "danger" : item.defaultRoute || item.wifi?.connected ? "success" : "neutral"}"
          >${this.connectionLabel(item)}</span
        >
      </div>
      <div class="adapter-facts">
        <span
          ><small>IP</small
          >${[...(item.ipv4Addresses || []), ...(item.ipv6Addresses || [])].map((ip) => `${ip.ipAddress}/${ip.prefixLength}`).join(", ") || "—"}</span
        ><span
          ><small>Gateway</small
          >${[...(item.ipv4Gateways || []), ...(item.ipv6Gateways || [])].join(", ") || "—"}</span
        ><span><small>Link</small>${item.linkSpeed || "—"}</span>
      </div>
      ${
        item.wifi
          ? html`<div class="inline-controls">
              <span
                >${item.wifi.ssid || "Wi-Fi"} · ${item.wifi.signal || "—"}</span
              ><select
                id=${`wifi-${item.interfaceIndex}`}
                ?disabled=${wifiProfiles === null}
              >
                ${wifiProfiles === null ? html`<option>${this.t("profilesLoading")}</option>` : (wifiProfiles || []).map((profile) => html`<option value=${profile} ?selected=${profile === item.wifi.currentProfile}>${profile}</option>`)}</select
              ><button
                class="secondary"
                ?disabled=${wifiProfiles === null || !(wifiProfiles || []).length || this.isBusy(`wifi:${item.interfaceIndex}`)}
                @click=${() => {
                  const profileName = this.querySelector(
                    `#wifi-${item.interfaceIndex}`,
                  ).value;
                  this.run(`wifi-preview:${item.interfaceIndex}`, () => api.previewWifi({ interfaceName: item.alias, profileName, action: "connect" }), []).then((preview) => {
                    if (preview) this.requestConfirmation([`${item.alias} · ${profileName}`, this.t("wifiConnectWarning"), ...(preview.warnings || [])], () => this.run(`wifi:${item.interfaceIndex}`, () => api.connectWifi(preview.wifi, preview.confirmationToken), ["interfaces"],), false);
                  });
                }}
              >
                ${this.t("connect")}</button
              ><button
                class="danger"
                ?disabled=${!item.wifi.connected || this.isBusy(`wifi:${item.interfaceIndex}`)}
                @click=${() => {
                  this.run(`wifi-preview:${item.interfaceIndex}`, () => api.previewWifi({ interfaceName: item.alias, action: "disconnect" }), []).then((preview) => {
                    if (preview) this.requestConfirmation([`${item.alias} ${this.t("disconnect")}`, ...(preview.warnings || [])], () => this.run(`wifi:${item.interfaceIndex}`, () => api.disconnectWifi(preview.wifi, preview.confirmationToken), ["interfaces"]));
                  });
                }}
              >
                ${this.t("disconnect")}
              </button>
            </div>`
          : nothing
      }
      <details>
        <summary>${this.t("technicalDetails")}</summary>
        <div class="details-grid">
          <span
            ><small>DNS</small
            >${(item.dnsServers || []).join(", ") || "—"}</span
          ><span
            ><small>Index / MAC</small>${item.interfaceIndex} /
            ${item.macAddress || "—"}</span
          >
        </div>
        <div class="metrics">
          ${this.renderMetric(item, "IPv4", "ipv4")}${this.renderMetric(item, "IPv6", "ipv6")}
        </div>
        ${!item.systemInterface ? html`<div class="inline-controls">${!this.privilege.isAdmin ? html`<span class="permission-note">${this.t("permissionHelp")}</span><button class="secondary compact" @click=${this.restartAsAdmin}>${this.t("restartAdmin")}</button>` : nothing}<button class="secondary" ?disabled=${!this.privilege.isAdmin || !disabled || this.isBusy(adapterBusy)} @click=${() => this.adapterAction(item, "enable")}>${this.t("enable")}</button><button class="danger" ?disabled=${!this.privilege.isAdmin || disabled || this.isBusy(adapterBusy)} @click=${() => this.adapterAction(item, "disable")}>${this.t("disable")}</button></div>` : nothing}
      </details>
    </article>`;
  }

  async adapterAction(item, action) {
    const key = `adapter:${item.interfaceIndex}`;
    const preview = await this.run(
      key,
      () => api.previewAdapter({ interfaceIndex: item.interfaceIndex, action }),
      [],
    );
    if (preview)
      this.requestConfirmation(
        [
          `${item.alias} (#${item.interfaceIndex})`,
          ...(preview.warnings || []),
        ],
        () =>
          this.run(key, () =>
            api.applyAdapter(
              { interfaceIndex: item.interfaceIndex, action },
              preview.confirmationToken,
            ),
          ),
      );
  }

  renderAdapters() {
    const adapters = this.interfaces().filter((item) =>
      `${item.alias} ${item.description} ${item.type}`
        .toLowerCase()
        .includes(this.adapterSearch.toLowerCase()),
    );
    return html`<section class="view-stack">
      <div class="toolbar">
        <label>${this.t("searchAdapters")}<input type="search" .value=${this.adapterSearch} @input=${(event) => { this.adapterSearch = event.target.value; this.requestUpdate(); }} /></label>
        <label class="toggle"
          ><input
            type="checkbox"
            .checked=${this.settings.showSystemInterfaces}
            @change=${(event) => this.updateSettings({ showSystemInterfaces: event.target.checked })}
          />${this.t("showSystem")}</label
        ><button class="secondary" @click=${() => this.refresh(["interfaces"])}>
          ${this.t("refresh")}
        </button>
      </div>
      <div class="adapter-grid">
        ${adapters.length ? adapters.map((item) => this.renderAdapter(item)) : html`<p class="empty">${this.t("noAdapters")}</p>`}
      </div>
    </section>`;
  }

  async addRoute(event) {
    event.preventDefault();
    if (!this.privilege.isAdmin)
      return this.notify(this.t("permissionHelp"), "error");
    const data = new FormData(event.currentTarget);
    const input = {
      destinationPrefix: data.get("destinationPrefix"),
      nextHop: data.get("nextHop"),
      interfaceIndex: data.get("interfaceIndex"),
      routeMetric: data.get("routeMetric"),
      persistent: data.get("persistent") === "on",
    };
    const preview = await this.run(
      "route:add",
      () => api.previewRoute(input),
      [],
    );
    if (preview)
      this.requestConfirmation(
        [
          `${preview.route.destinationPrefix} → ${preview.route.nextHop}`,
          ...(preview.warnings || []),
        ],
        () =>
          this.run("route:add", () =>
            api.addRoute(preview.route, preview.confirmationToken),
          ),
      );
  }

  renderRoutes() {
    const rows = this.routeRows();
    return html`<section class="view-stack">
      <div class="toolbar route-filters">
        <label
          >${this.t("routeSearch")}<input
            type="search"
            .value=${this.routeSearch}
            @input=${(event) => {
              this.routeSearch = event.target.value;
              this.requestUpdate();
            }} /></label
        ><label
          >${this.t("type")}<select
            .value=${this.routeFamily}
            @change=${(event) => {
              this.routeFamily = event.target.value;
              this.requestUpdate();
            }}
          >
            <option value="IPv4">IPv4</option>
            <option value="IPv6">IPv6</option>
            <option value="all">All</option>
          </select></label
        ><label class="toggle"
          ><input
            type="checkbox"
            .checked=${this.settings.showAllRoutes}
            @change=${(event) => this.updateSettings({ showAllRoutes: event.target.checked })}
          />${this.t("showAllRoutes")}</label
        ><button class="secondary" @click=${() => this.refresh(["routes"])}>
          ${this.t("refresh")}
        </button>
      </div>
      <div class="table-wrap">
        <table>
          <thead>
            <tr>
              <th>Family</th>
              <th>${this.t("destination")}</th>
              <th>${this.t("nextHop")}</th>
              <th>${this.t("interface")}</th>
              <th>${this.t("metric")}</th>
              <th>${this.t("type")}</th>
              <th>${this.t("actions")}</th>
            </tr>
          </thead>
          <tbody>
            ${
              rows.length
                ? rows.map(
                    (route) =>
                      html`<tr>
                        <td>${route.addressFamily}</td>
                        <td>${route.destinationPrefix}</td>
                        <td>${route.onLink ? "On-link" : route.nextHop}</td>
                        <td>
                          ${route.interfaceAlias} (#${route.interfaceIndex})
                        </td>
                        <td>${route.effectiveMetric ?? route.routeMetric}</td>
                        <td>
                          ${route.persistent ? "Persistent" : "Temporary"}${route.systemManaged ? " / System" : ""}
                        </td>
                        <td>
                          ${route.removable ? html`<button class="danger compact" ?disabled=${!this.privilege.isAdmin || this.isBusy(`route:remove:${route.destinationPrefix}`)} @click=${() => this.removeRoute(route)}>${this.t("remove")}</button>` : "—"}
                        </td>
                      </tr>`,
                  )
                : html`<tr>
                    <td colspan="7" class="empty">${this.t("noRoutes")}</td>
                  </tr>`
            }
          </tbody>
        </table>
      </div>
      <form class="panel route-form" @submit=${this.addRoute}>
        <h2>${this.t("addRoute")}</h2>
        <label
          >${this.t("destination")}<input
            name="destinationPrefix"
            placeholder="172.16.0.0/12"
            required /></label
        ><label
          >${this.t("nextHop")}<input
            name="nextHop"
            placeholder="192.168.3.1"
            required /></label
        ><label
          >${this.t("interface")}<select name="interfaceIndex">
            ${this.interfaces()
              .filter((item) => item.metrics?.ipv4)
              .map(
                (item) =>
                  html`<option value=${item.interfaceIndex}>
                    ${item.alias} (#${item.interfaceIndex})
                  </option>`,
              )}
          </select></label
        ><label
          >${this.t("metric")}<input
            name="routeMetric"
            type="number"
            min="1"
            max="9999"
            value="5"
            required /></label
        ><label class="toggle"
          ><input
            name="persistent"
            type="checkbox"
          />${this.t("persistent")}</label
        ><button
          class="primary"
          ?disabled=${!this.privilege.isAdmin || this.isBusy("route:add")}
        >
          ${this.isBusy("route:add") ? this.t("working") : this.t("addRoute")}</button
        >${!this.privilege.isAdmin ? html`<p class="permission-note">${this.t("permissionHelp")}</p>` : nothing}
      </form>
    </section>`;
  }

  async removeRoute(route) {
    const key = `route:remove:${route.destinationPrefix}`;
    const preview = await this.run(
      key,
      () => api.previewRemoveRoute(route),
      [],
    );
    if (preview)
      this.requestConfirmation(
        [
          `${route.destinationPrefix} → ${route.nextHop}`,
          ...(preview.warnings || []),
        ],
        () =>
          this.run(key, () =>
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
          ),
      );
  }

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
      ]);
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
      );
    try {
      this.config = { ...this.config, ...(await api.profiles()) };
    } catch {
      /* refresh is best effort */
    }
    this.requestUpdate();
  }

  restoreProfile(item, recent = false) {
    const key = `${recent ? "recent" : "profile"}:${item.id}`;
    this.run(
      key,
      () => (recent ? api.recentPreview(item.id) : api.profilePreview(item.id)),
      [],
    ).then((preview) => {
      if (preview)
        this.requestConfirmation([preview.summary], () =>
          this.run(key, () =>
            recent
              ? api.recentRestore(item.id, preview.confirmationToken)
              : api.profileRestore(item.id, preview.confirmationToken),
          ),
        );
    });
  }

  renderProfiles() {
    return html`<section class="view-stack">
      <div class="toolbar">
        <button class="primary" @click=${() => this.openNameDialog("snapshot")}>
          ${this.t("saveSnapshot")}</button
        ><button
          class="secondary"
          ?disabled=${!this.network.overview || !this.network.routes}
          @click=${() => this.openNameDialog("profile")}
        >
          ${this.t("saveProfile")}
        </button>
      </div>
      <section class="panel">
        <h2>${this.t("savedProfiles")}</h2>
        <div class="list">
          ${
            this.config.profiles?.length
              ? this.config.profiles.map(
                  (item) =>
                    html`<div class="list-row">
                      <div>
                        <strong>${item.name}</strong
                        ><small
                          >${formatTime(item.updatedAt || item.createdAt, this.settings.language)}</small
                        >
                      </div>
                      <div>
                        <button
                          class="secondary"
                          ?disabled=${!this.privilege.isAdmin}
                          @click=${() => this.restoreProfile(item)}
                        >
                          ${this.t("restore")}</button
                        ><button
                          class="danger"
                          @click=${async () => {
                            await this.run(`delete:${item.id}`, () => api.deleteProfile(item.id), []);
                            await this.refreshConfig().catch(() => {});
                          }}
                        >
                          ${this.t("delete")}
                        </button>
                      </div>
                    </div>`,
                )
              : html`<p class="empty">—</p>`
          }
        </div>
      </section>
      <section class="panel">
        <h2>${this.t("recentSnapshots")}</h2>
        <div class="list">
          ${
            this.config.recentSnapshots?.length
              ? this.config.recentSnapshots.map(
                  (item) =>
                    html`<div class="list-row">
                      <div>
                        <strong>${item.name}</strong
                        ><small
                          >${formatTime(item.lastUsedAt, this.settings.language)}</small
                        >
                      </div>
                      <button
                        class="secondary"
                        ?disabled=${!this.privilege.isAdmin}
                        @click=${() => this.restoreProfile(item, true)}
                      >
                        ${this.t("restore")}
                      </button>
                    </div>`,
                )
              : html`<p class="empty">—</p>`
          }
        </div>
      </section>
    </section>`;
  }

  renderDiagnostics() {
    const changes = this.diagnostics?.changes || {};
    const findings = [
      ...(changes.interfaces || []).map((item) => `${item.alias || item.interfaceIndex}: ${this.t("configurationChange")}`),
      ...(changes.staticRoutes || []).map((route) => `${route.destinationPrefix}: ${this.t("staticRouteFound")}`),
    ];
    return html`<section class="view-stack">
      <div class="toolbar">
        <button
          class="primary"
          ?disabled=${this.isBusy("diagnostics")}
          @click=${() =>
            this.run(
              "diagnostics",
              async () => {
                this.diagnostics = await api.diagnostics();
                this.requestUpdate();
                return { message: this.t("operationComplete") };
              },
              [],
            )}
        >
          ${this.isBusy("diagnostics") ? this.t("working") : this.t("generateDiagnostics")}</button
        ><button
          class="secondary"
          ?disabled=${!this.diagnostics}
          @click=${async () => {
            await navigator.clipboard.writeText(
              JSON.stringify(this.diagnostics, null, 2),
            );
            this.notify(this.t("operationComplete"), "success");
          }}
        >
          ${this.t("copyReport")}
        </button>
      </div>
      ${this.diagnostics ? html`<section class="panel"><h2>${this.t("diagnosticSummary")}</h2>${findings.length ? html`<ul class="findings">${findings.map((finding) => html`<li>${finding}</li>`)}</ul>` : html`<p>${this.t("noChanges")}</p>`}<details><summary>${this.t("technicalDetails")}</summary><pre class="diagnostics">${JSON.stringify(this.diagnostics, null, 2)}</pre></details></section>` : html`<p class="empty">${this.t("diagnosticsIdle")}</p>`}
    </section>`;
  }

  renderMaintenance() {
    return html`<section class="view-stack">
      <section class="panel danger-zone">
        <h2>${this.t("undoLastChange")}</h2>
        <p>
          ${this.config.hasBaseline ? this.t("changes") : this.t("noBaseline")}
        </p>
        <button
          class="warning"
          ?disabled=${!this.config.hasBaseline || !this.privilege.isAdmin}
          @click=${async () => {
            const preview = await this.run(
              "reset-preview",
              () => api.resetPreview(),
              [],
            );
            if (preview)
              this.requestConfirmation([this.t("undoLastChange"), ...Object.keys(preview.scope || {}).map((key) => `${this.t("affected")}: ${key}`)], () =>
                this.run("reset", () => api.reset(preview.confirmationToken)),
              );
          }}
        >
          ${this.t("undoLastChange")}
        </button>
      </section>
      <section class="panel danger-zone">
        <h2>${this.t("restoreDefaults")}</h2>
        <p>${this.t("defaultsHelp")}</p>
        <div class="default-interface-list">
          ${this.interfaces()
            .filter((item) => !item.systemInterface)
            .map(
              (item) =>
                html`<label class="toggle"
                  ><input
                    data-default-adapter
                    type="checkbox"
                    value=${item.interfaceIndex}
                    checked
                  />${item.alias}</label
                >`,
            )}
        </div>
        <label class="toggle"
          ><input
            id="remove-managed"
            type="checkbox"
            checked
          />${this.t("removeManagedRoutes")}</label
        ><button
          class="danger"
          ?disabled=${!this.privilege.isAdmin}
          @click=${async () => {
            const selection = {
              interfaceIndexes: [
                ...this.querySelectorAll("input[data-default-adapter]:checked"),
              ].map((input) => Number(input.value)),
              removeManagedRoutes:
                this.querySelector("#remove-managed").checked,
            };
            const preview = await this.run(
              "defaults-preview",
              () => api.defaultsPreview(selection),
              [],
            );
            if (preview)
              this.requestConfirmation(
                [
                  this.t("restoreDefaults"),
                  ...(preview.interfaces || []).map((item) => item.alias),
                ],
                () =>
                  this.run("defaults", () =>
                    api.defaults(selection, preview.confirmationToken),
                  ),
              );
          }}
        >
          ${this.t("restoreDefaults")}
        </button>
      </section>
    </section>`;
  }

  renderView() {
    return {
      overview: this.renderOverview,
      adapters: this.renderAdapters,
      routes: this.renderRoutes,
      profiles: this.renderProfiles,
      diagnostics: this.renderDiagnostics,
      maintenance: this.renderMaintenance,
    }[this.view].call(this);
  }

  renderOverlay() {
    if (this.confirmation)
      return html`<div class="modal-backdrop" role="presentation">
        <section
          class="modal"
          role="dialog"
          aria-modal="true"
          aria-labelledby="confirm-heading"
        >
          <h2 id="confirm-heading">${this.t("confirm")}</h2>
          <ul>
            ${this.confirmation.lines.map((line) => html`<li>${line}</li>`)}
          </ul>
          <div class="modal-actions">
            <button class="secondary" @click=${this.closeConfirmation}>
              ${this.t("cancel")}</button
            ><button
              class=${this.confirmation.destructive ? "danger" : "primary"}
              @click=${this.confirm}
            >
              ${this.t("confirm")}
            </button>
          </div>
        </section>
      </div>`;
    if (this.nameDialog)
      return html`<div class="modal-backdrop" role="presentation">
        <form
          class="modal"
          @submit=${this.saveNamed}
          role="dialog"
          aria-modal="true"
        >
          <h2>
            ${this.nameDialog.kind === "snapshot" ? this.t("snapshotName") : this.t("configurationName")}
          </h2>
          <label><input name="name" required autofocus /></label>
          <div class="modal-actions">
            <button
              type="button"
              class="secondary"
              @click=${this.closeNameDialog}
            >
              ${this.t("cancel")}</button
            ><button class="primary">${this.t("save")}</button>
          </div>
        </form>
      </div>`;
    return nothing;
  }

  render() {
    return html`<div class="app-shell">
      ${this.renderNav()}
      <main>
        <div class="mobile-nav">
          ${views.map(
            (view) =>
              html`<button
                class=${this.view === view ? "selected" : ""}
                @click=${() => {
                  this.view = view;
                  this.requestUpdate();
                }}
              >
                ${this.t(view)}
              </button>`,
          )}
        </div>
        ${this.renderHeader()}${this.message ? html`<div class="notice ${this.message.kind}" role="status"><span>${this.message.text}</span><button class="icon-button" aria-label=${this.t("close")} @click=${this.dismissMessage}>×</button></div>` : nothing}${this.renderView()}
      </main>
      ${this.renderOverlay()}
    </div>`;
  }
}

customElements.define("network-manager-app", NetworkManagerApp);
