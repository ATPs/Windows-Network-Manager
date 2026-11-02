import { html, nothing } from "lit";
import { adapterIconName, icon } from "../icons.js";
import {
  autoGuidKey,
  autoResultText,
  formatTime,
  hasManualMetric,
  isAdapterDisabled,
  isAdapterOnline,
  parseSignal,
  primaryIpv4,
  sortAdapters,
} from "../format.js";
import {
  buttonContent,
  chip,
  copyButton,
  emptyState,
  fact,
  pill,
  signalBars,
  skeletonRows,
  switchInput,
} from "../ui.js";

const typeFilter = (item) => {
  if (item.wifi) return "wifi";
  const type = String(item.type || "").toLowerCase();
  if (type.includes("ethernet")) return "ethernet";
  if (type.includes("vpn") || type.includes("tunnel")) return "vpn";
  return "other";
};

const filters = {
  all: () => true,
  connected: isAdapterOnline,
  wifi: (item) => typeFilter(item) === "wifi",
  ethernet: (item) => typeFilter(item) === "ethernet",
  vpn: (item) => typeFilter(item) === "vpn",
  other: (item) => typeFilter(item) === "other",
  manual: hasManualMetric,
  disabled: isAdapterDisabled,
};

function renderFilterChips(app, adapters) {
  const labels = {
    all: "filterAll",
    connected: "connected",
    wifi: "filterWifi",
    ethernet: "filterEthernet",
    vpn: "filterVpn",
    other: "filterOther",
    manual: "manualMetrics",
    disabled: "disabled",
  };
  return html`<div class="chip-filters" role="group" aria-label=${app.t("filter")}>
    ${Object.keys(filters).map((key) => {
      const count = adapters.filter(filters[key]).length;
      if (!count && key !== "all" && key !== app.adapterFilter) return nothing;
      return html`<button
        class="filter-chip ${app.adapterFilter === key ? "selected" : ""}"
        aria-pressed=${String(app.adapterFilter === key)}
        @click=${() => {
          app.adapterFilter = key;
          app.requestUpdate();
        }}
      >
        ${app.t(labels[key])}<span class="count">${count}</span>
      </button>`;
    })}
  </div>`;
}

function renderMetric(app, item, family, key) {
  const metric = item.metrics?.[key];
  if (!metric) return nothing;
  const id = `metric-${item.interfaceIndex}-${key}`;
  const busyKey = `metric:${item.interfaceIndex}:${family}`;
  const busy = app.isBusy(busyKey);
  return html`<div class="metric-control">
    <strong>${family}</strong>
    <label class="check"
      ><input
        id=${`${id}-auto`}
        type="checkbox"
        .checked=${Boolean(metric.automaticMetric)}
        @change=${(event) => {
          const input = app.querySelector(`#${id}-value`);
          if (input) input.disabled = event.target.checked;
        }}
      />${app.t("automaticMetric")}</label
    >
    <input
      id=${`${id}-value`}
      type="number"
      min="1"
      max="9999"
      aria-label=${`${family} ${app.t("metric")}`}
      .value=${String(metric.interfaceMetric)}
      ?disabled=${Boolean(metric.automaticMetric)}
    />
    <button
      class="standard"
      ?disabled=${!app.privilege.isAdmin || busy}
      @click=${() => app.applyMetric(item, family, key)}
    >
      ${buttonContent(busy, null, app.t("applyMetric"))}
    </button>
  </div>`;
}

function autoSummary(app, rule, status) {
  if (!rule) return app.t("autoOffSummary");
  const state = status?.state || (rule.enabled ? "waiting" : "disabled");
  const parts = [
    `${rule.enabled ? app.t("enabled") : app.t("disabled")} → ${rule.profileName}`,
    app.tf("everySeconds", { seconds: rule.intervalSeconds }),
  ];
  if (status?.lastCheck)
    parts.push(`${app.t("wifiAutoLastCheck")} ${formatTime(status.lastCheck, app.settings.language)}`);
  if (status?.lastResult) parts.push(app.wifiAutoResultText(status.lastResult));
  return html`${pill(app.wifiAutoStateClass(state), app.wifiAutoStateText(state))}<span
      >${parts.join(" · ")}</span
    >`;
}

export function renderWifiAutoControl(app, item, configuredAdapter = null) {
  const matchingAdapter = item?.alias
    ? (app.wifiAuto.adapters || []).find(
        (adapter) =>
          String(adapter.interfaceName || "").toLowerCase() === String(item.alias).toLowerCase(),
      )
    : null;
  const adapterInfo = configuredAdapter || matchingAdapter;
  const guid = app.wifiAutoGuid(item, adapterInfo);
  if (!guid)
    return html`<div class="auto-connect">
      <div class="auto-summary">${icon("info")}<span>${app.t("wifiAutoGuidHelp")}</span></div>
    </div>`;
  const rule = app.wifiAutoRule(guid);
  const status = app.wifiAutoStatus(guid);
  const resolvedAdapterInfo = adapterInfo || app.wifiAutoAdapter(guid);
  const profiles = app.wifiAutoProfiles(guid, item, resolvedAdapterInfo);
  const key = autoGuidKey(guid);
  const busyKey = `wifi-auto:${key}`;
  const editing = app.wifiAutoEditing.has(key);
  const missing = !item;
  const name = item?.alias || resolvedAdapterInfo?.interfaceName || app.t("wifiAutoConfiguredAdapter");
  const draft = editing ? app.wifiAutoDraft(guid, item, resolvedAdapterInfo) : null;
  return html`<div class="auto-connect">
    <div class="auto-summary">
      ${missing ? html`<strong>${name}</strong>` : nothing} ${autoSummary(app, rule, status)}
      <div class="auto-actions">
        ${!editing
          ? html`<button class="standard compact" @click=${() => app.toggleWifiAutoEditor(guid, true)}>
              ${icon(rule ? "edit" : "plus")}${rule ? app.t("edit") : app.t("autoSetUp")}
            </button>`
          : nothing}
        ${rule?.enabled
          ? html`<button
              class="subtle compact"
              ?disabled=${app.isBusy(busyKey)}
              @click=${() => app.disableWifiAuto(guid)}
            >
              ${buttonContent(app.isBusy(busyKey), "power", app.t("wifiAutoDisable"))}
            </button>`
          : nothing}
        ${rule
          ? html`<button
              class="icon-button subtle danger-text"
              title=${app.t("remove")}
              aria-label=${app.t("remove")}
              ?disabled=${app.isBusy(busyKey)}
              @click=${() => app.removeWifiAuto(guid, `${name} · ${rule.profileName}`)}
            >
              ${icon("trash")}
            </button>`
          : nothing}
      </div>
    </div>
    ${status?.error
      ? html`<p class="inline-error">${icon("error")}${status.error}</p>`
      : nothing}
    ${resolvedAdapterInfo?.discoveryError
      ? html`<p class="inline-error">
          ${icon("error")}${app.t("wifiAutoDiscoveryError")}: ${autoResultText(resolvedAdapterInfo.discoveryError)}
        </p>`
      : nothing}
    ${missing ? html`<small class="muted mono">${guid}</small>` : nothing}
    ${editing
      ? html`<div class="auto-editor">
          <p class="help-text">${app.t("wifiAutoHelp")}</p>
          <div class="form-row">
            <label class="field"
              >${app.t("wifiAutoProfile")}
              <select
                .value=${draft.profileName}
                ?disabled=${!profiles.length || app.isBusy(busyKey)}
                @change=${(event) => app.updateWifiAutoDraft(guid, { profileName: event.target.value })}
              >
                ${profiles.length
                  ? html`<option value="">${app.t("wifiAutoChooseProfile")}</option>${profiles.map(
                        (profile) =>
                          html`<option value=${profile} ?selected=${profile === draft.profileName}>
                            ${profile}
                          </option>`,
                      )}`
                  : html`<option value="">${app.t("wifiAutoNoProfiles")}</option>`}
              </select>
            </label>
            <label class="field narrow"
              >${app.t("wifiAutoInterval")}
              <input
                type="number"
                min="5"
                max="86400"
                step="1"
                .value=${String(draft.intervalSeconds)}
                ?disabled=${app.isBusy(busyKey)}
                @input=${(event) => app.updateWifiAutoDraft(guid, { intervalSeconds: event.target.value })}
              />
            </label>
            ${switchInput(
              draft.enabled,
              app.t("wifiAutoEnabled"),
              (checked) => app.updateWifiAutoDraft(guid, { enabled: checked }),
              app.isBusy(busyKey),
            )}
          </div>
          <div class="button-row">
            <button
              class="accent"
              ?disabled=${!profiles.length ||
              !draft.profileName ||
              app.isBusy(`${busyKey}:preview`) ||
              app.isBusy(busyKey)}
              @click=${() => app.saveWifiAuto(draft)}
            >
              ${buttonContent(app.isBusy(`${busyKey}:preview`), null, app.t("save"))}
            </button>
            <button class="standard" @click=${() => app.toggleWifiAutoEditor(guid, false)}>
              ${app.t("cancel")}
            </button>
          </div>
        </div>`
      : nothing}
  </div>`;
}

function renderWifiSection(app, item) {
  const wifi = item.wifi;
  const profiles = app.wifiProfiles.get(item.alias);
  const signal = wifi.signalPercent ?? parseSignal(wifi.signal);
  const busy = app.isBusy(`wifi:${item.interfaceIndex}`) || app.isBusy(`wifi-preview:${item.interfaceIndex}`);
  const channel = [wifi.band, wifi.channel ? `${app.t("channel")} ${wifi.channel}` : ""]
    .filter(Boolean)
    .join(" · ");
  return html`<section class="detail-section span-2">
    <h4>${icon("wifi")}Wi-Fi</h4>
    <div class="fact-grid">
      <div class="fact">
        <small>${app.t("network")}</small>
        <span class="fact-value"
          >${wifi.connected
            ? html`${signalBars(signal, `${app.t("signal")} ${signal ?? "—"}%`)}<span
                  >${wifi.ssid || "—"}${signal != null ? ` · ${signal}%` : ""}</span
                >`
            : html`<span>${app.t("disconnected")}</span>`}</span
        >
      </div>
      ${channel ? fact(app.t("bandChannel"), channel) : nothing}
      ${wifi.radioType ? fact(app.t("radioType"), wifi.radioType) : nothing}
      ${wifi.authentication
        ? fact(app.t("security"), [wifi.authentication, wifi.cipher].filter(Boolean).join(" · "))
        : nothing}
      ${wifi.receiveRate || wifi.transmitRate
        ? fact(app.t("linkRate"), `↓ ${wifi.receiveRate || "—"} · ↑ ${wifi.transmitRate || "—"} Mbps`)
        : nothing}
      ${wifi.bssid ? fact("BSSID", wifi.bssid, { copy: wifi.bssid, app }) : nothing}
    </div>
    <div class="form-row wifi-controls">
      <label class="field"
        >${app.t("savedProfile")}
        <select id=${`wifi-${item.interfaceIndex}`} ?disabled=${profiles == null || busy}>
          ${profiles == null
            ? html`<option>${app.t("profilesLoading")}</option>`
            : profiles.length
              ? profiles.map(
                  (profile) =>
                    html`<option value=${profile} ?selected=${profile === wifi.currentProfile}>
                      ${profile}
                    </option>`,
                )
              : html`<option value="">${app.t("wifiAutoNoProfiles")}</option>`}
        </select>
      </label>
      <div class="button-row">
        <button
          class="standard"
          ?disabled=${!profiles?.length || busy}
          @click=${() => app.connectWifi(item)}
        >
          ${buttonContent(app.isBusy(`wifi-preview:${item.interfaceIndex}`), "wifi", app.t("connect"))}
        </button>
        <button
          class="subtle danger-text"
          ?disabled=${!wifi.connected || busy}
          @click=${() => app.disconnectWifi(item)}
        >
          ${buttonContent(app.isBusy(`wifi:${item.interfaceIndex}`), "close", app.t("disconnect"))}
        </button>
      </div>
    </div>
    <h5>${app.t("wifiAutoConnect")}</h5>
    ${renderWifiAutoControl(app, item)}
  </section>`;
}

function addressList(app, addresses) {
  if (!addresses?.length) return html`<span>—</span>`;
  return html`<span class="address-list"
    >${addresses.map(
      (ip) =>
        html`<span class="fact-value"
          ><span class="mono">${ip.ipAddress}/${ip.prefixLength}</span>${copyButton(app, ip.ipAddress)}</span
        >`,
    )}</span
  >`;
}

function renderQuickFixes(app, item) {
  const fixButton = (action, iconName) => {
    const key = `fix:${action}:${item.interfaceIndex}`;
    const busy = app.isBusy(key) || app.isBusy(`${key}:preview`);
    return html`<button class="standard" ?disabled=${!app.privilege.isAdmin || busy} @click=${() => app.quickFix(action, item)}>
      ${buttonContent(busy, iconName, app.t(action))}
    </button>`;
  };
  return html`<div class="advanced-row">
    <p class="help-text">${app.t("quickFixHelp")}</p>
    <span class="button-row">
      ${item.dhcpEnabled ? fixButton("renewDhcp", "refresh") : nothing} ${fixButton("restartAdapter", "power")}
    </span>
  </div>`;
}

function renderAdapterBody(app, item) {
  const disabled = isAdapterDisabled(item);
  const adapterBusy = app.isBusy(`adapter:${item.interfaceIndex}`);
  const connectivity = app.connectivityInfo(item);
  return html`<div class="adapter-body">
    <section class="detail-section">
      <h4>${icon("globe")}${app.t("connection")}</h4>
      <div class="fact-grid two">
        <div class="fact wide"><small>${app.t("ipv4Address")}</small>${addressList(app, item.ipv4Addresses)}</div>
        <div class="fact wide"><small>${app.t("ipv6Address")}</small>${addressList(app, item.ipv6Addresses)}</div>
        ${fact(app.t("gateway"), [...(item.ipv4Gateways || []), ...(item.ipv6Gateways || [])].join(", "))}
        ${fact(app.t("dns"), (item.dnsServers || []).join(", "), {
          copy: (item.dnsServers || []).join(", "),
          app,
        })}
        ${fact(app.t("connectivity"), connectivity?.text)}
        ${fact("DHCP", item.dhcpEnabled == null ? null : item.dhcpEnabled ? app.t("enabled") : app.t("disabled"))}
        ${fact(app.t("macAddress"), item.macAddress, { copy: item.macAddress, app })}
        ${fact(app.t("interfaceIndex"), `#${item.interfaceIndex}`)}
        ${fact(app.t("linkSpeed"), item.linkSpeed)}
        ${fact(app.t("operationalStatus"), item.operationalStatus)}
      </div>
    </section>
    <section class="detail-section">
      <h4>${icon("routes")}${app.t("priority")}</h4>
      <p class="help-text">${app.t("metricHelp")}</p>
      <div class="metrics">
        ${renderMetric(app, item, "IPv4", "ipv4")}${renderMetric(app, item, "IPv6", "ipv6")}
      </div>
      ${!item.metrics?.ipv4 && !item.metrics?.ipv6 ? html`<p class="muted">—</p>` : nothing}
      ${!app.privilege.isAdmin ? html`<p class="permission-note">${icon("shield")}${app.t("permissionHelp")}</p>` : nothing}
    </section>
    ${item.wifi ? renderWifiSection(app, item) : nothing}
    ${item.managedBy
      ? html`<section class="detail-section span-2 advanced">
          <h4>${icon("shield")}${app.managedLabel(item.managedBy)}</h4>
          <p class="help-text">
            ${app.tf("protectedAdapterHelp", { owner: app.ownerLabel(item.managedBy) })}
          </p>
        </section>`
      : !item.systemInterface
      ? html`<section class="detail-section span-2 advanced">
          <h4>${icon("power")}${app.t("advanced")}</h4>
          <div class="advanced-row">
            <p class="help-text">${app.t(disabled ? "enableHelp" : "disableHelp")}</p>
            ${disabled
              ? html`<button
                  class="standard"
                  ?disabled=${!app.privilege.isAdmin || adapterBusy}
                  @click=${() => app.adapterAction(item, "enable")}
                >
                  ${buttonContent(adapterBusy, "power", app.t("enable"))}
                </button>`
              : html`<button
                  class="danger"
                  ?disabled=${!app.privilege.isAdmin || adapterBusy}
                  @click=${() => app.adapterAction(item, "disable")}
                >
                  ${buttonContent(adapterBusy, "power", app.t("disable"))}
                </button>`}
          </div>
          ${disabled ? nothing : renderQuickFixes(app, item)}
        </section>`
      : nothing}
  </div>`;
}

function renderAdapter(app, item) {
  const key = String(item.interfaceIndex);
  const expanded = app.expandedAdapters.has(key);
  const disabled = isAdapterDisabled(item);
  const online = isAdapterOnline(item);
  const rule = item.wifi ? app.wifiAutoRule(app.wifiAutoGuid(item)) : null;
  const signal = item.wifi ? item.wifi.signalPercent ?? parseSignal(item.wifi.signal) : null;
  const toggle = () => {
    expanded ? app.expandedAdapters.delete(key) : app.expandedAdapters.add(key);
    app.requestUpdate();
  };
  return html`<article class="card adapter-card ${expanded ? "expanded" : ""}" data-adapter=${key}>
    <button class="adapter-head" aria-expanded=${String(expanded)} @click=${toggle}>
      <span class="adapter-icon ${online ? "online" : disabled ? "off" : ""}">${icon(adapterIconName(item))}</span>
      <span class="adapter-title">
        <strong>${item.alias}</strong>
        <small>${item.description || item.type}</small>
      </span>
      <span class="adapter-summary">
        ${item.wifi?.connected
          ? html`<span class="inline-fact"
              >${signalBars(signal, `${app.t("signal")} ${signal ?? "—"}%`)}${item.wifi.ssid}</span
            >`
          : nothing}
        <span class="inline-fact mono">${primaryIpv4(item) || "—"}</span>
        ${item.linkSpeed && item.linkSpeed !== "—"
          ? html`<span class="inline-fact muted">${item.linkSpeed}</span>`
          : nothing}
      </span>
      <span class="adapter-badges">
        ${hasManualMetric(item) ? chip(app.t("manualMetric"), "warning") : nothing}
        ${rule?.enabled ? chip(app.t("autoOn"), "accent") : nothing}
        ${item.systemInterface ? chip(app.t("system")) : nothing}
        ${item.managedBy ? chip(app.managedLabel(item.managedBy), "accent") : nothing}
        ${pill(disabled ? "danger" : online ? "success" : "neutral", app.connectionLabel(item))}
      </span>
      ${icon(expanded ? "chevronUp" : "chevronDown", "muted")}
    </button>
    ${expanded ? renderAdapterBody(app, item) : nothing}
  </article>`;
}

function renderMissingWifiAutoRules(app) {
  const current = new Set(
    app
      .interfaces()
      .filter((item) => item.wifi)
      .map((item) => autoGuidKey(app.wifiAutoGuid(item))),
  );
  const missing = (app.wifiAuto.rules || []).filter(
    (rule) => !current.has(autoGuidKey(rule.adapterGuid)),
  );
  if (!missing.length) return nothing;
  return html`<section class="card">
    <div class="card-header">
      <div>
        <h2>${app.t("wifiAutoConfigured")}</h2>
        <p>${app.t("wifiAutoMissingHelp")}</p>
      </div>
    </div>
    ${missing.map((rule) =>
      renderWifiAutoControl(app, null, app.wifiAutoAdapter(rule.adapterGuid) || rule),
    )}
  </section>`;
}

export function renderAdapters(app) {
  const all = app.interfaces();
  const search = app.adapterSearch.trim().toLowerCase();
  const adapters = sortAdapters(
    all.filter(
      (item) =>
        (filters[app.adapterFilter] || filters.all)(item) &&
        `${item.alias} ${item.description} ${item.type} ${item.wifi?.ssid || ""} ${primaryIpv4(item) || ""}`
          .toLowerCase()
          .includes(search),
    ),
  );
  return html`<div class="stack">
    <div class="toolbar">
      <label class="search-box"
        >${icon("search")}<input
          type="search"
          data-page-search
          placeholder=${app.t("searchAdapters")}
          aria-label=${app.t("searchAdapters")}
          .value=${app.adapterSearch}
          @input=${(event) => {
            app.adapterSearch = event.target.value;
            app.requestUpdate();
          }}
      /></label>
      ${switchInput(app.settings.showSystemInterfaces, app.t("showSystem"), (checked) =>
        app.updateSettings({ showSystemInterfaces: checked }),
      )}
    </div>
    ${renderFilterChips(app, all)}
    ${app.wifiAuto.discoveryError
      ? html`<div class="notice error">
          ${icon("error")}${app.t("wifiAutoDiscoveryError")}: ${autoResultText(app.wifiAuto.discoveryError)}
        </div>`
      : nothing}
    ${!app.network.overview
      ? html`<section class="card">${skeletonRows(4)}</section>`
      : adapters.length
        ? html`<div class="adapter-list">${adapters.map((item) => renderAdapter(app, item))}</div>`
        : html`<section class="card">${emptyState("adapters", app.t("noAdapters"))}</section>`}
    ${renderMissingWifiAutoRules(app)}
  </div>`;
}
