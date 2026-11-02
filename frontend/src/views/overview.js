import { html, nothing } from "lit";
import { adapterIconName, icon } from "../icons.js";
import {
  formatTime,
  hasManualMetric,
  isAdapterDisabled,
  isAdapterOnline,
  parseSignal,
  primaryIpv4,
  sortAdapters,
  sortDefaultCandidates,
} from "../format.js";
import { cardHeader, chip, emptyState, fact, pill, signalBars, skeletonRows } from "../ui.js";

function renderHero(app, active) {
  if (!app.network.overview)
    return html`<section class="card hero">${skeletonRows(2)}</section>`;
  if (!active)
    return html`<section class="card hero">
      ${emptyState("globe", app.t("noActivePath"), app.t("noActivePathHelp"))}
    </section>`;
  const connectivity = app.connectivityInfo(active);
  const signal = active.wifi ? active.wifi.signalPercent ?? parseSignal(active.wifi.signal) : null;
  const ipv4 = primaryIpv4(active);
  const gateway = (active.ipv4Gateways || [])[0] || (active.ipv6Gateways || [])[0];
  const dns = (active.dnsServers || []).slice(0, 2).join(", ");
  return html`<section class="card hero">
    <div class="hero-main">
      <span class="hero-icon">${icon(adapterIconName(active))}</span>
      <div class="hero-title">
        <small>${app.t("activePath")}</small>
        <h2>${active.wifi?.ssid || active.alias}</h2>
        <p>
          ${active.wifi?.ssid ? `${active.alias} · ` : ""}${active.description || active.type}
        </p>
      </div>
      <div class="hero-badges">
        ${connectivity
          ? pill(connectivity.kind, connectivity.text, connectivity.icon)
          : pill("success", app.connectionLabel(active), "success")}
        ${active.networkName && active.networkName !== active.wifi?.ssid
          ? chip(active.networkName)
          : nothing}
        ${active.networkCategory ? chip(app.t(`category${active.networkCategory}`)) : nothing}
        ${active.managedBy ? chip(app.managedLabel(active.managedBy), "accent") : nothing}
        <button class="subtle compact" ?disabled=${app.isBusy("tools:connectivity")} @click=${() => app.openTools()}>
          ${icon("diagnostics")}${app.t("testConnection")}
        </button>
      </div>
    </div>
    <div class="fact-grid">
      ${fact(app.t("ipv4Address"), ipv4, { copy: ipv4?.split("/")[0], app })}
      ${fact(app.t("gateway"), gateway, { copy: gateway, app })}
      ${fact(app.t("dns"), dns, { copy: dns, app })}
      ${active.wifi
        ? html`<div class="fact">
            <small>${app.t("signal")}</small>
            <span class="fact-value"
              >${signalBars(signal, `${app.t("signal")} ${signal ?? "—"}%`)}<span
                >${signal != null ? `${signal}%` : "—"}${active.wifi.band
                  ? ` · ${active.wifi.band}`
                  : ""}</span
              ></span
            >
          </div>`
        : fact(app.t("linkSpeed"), active.linkSpeed)}
    </div>
  </section>`;
}

function renderDefaultRoutes(app) {
  const routes = app.network.routes;
  const candidates = sortDefaultCandidates(
    routes?.defaultCandidates || (routes?.routes || []).filter((route) => route.defaultRoute),
  );
  const families = ["IPv4", "IPv6"]
    .map((family) => [family, candidates.filter((route) => route.addressFamily === family)])
    .filter(([, list]) => list.length);
  return html`<section class="card">
    ${cardHeader(app.t("defaultPriority"), app.t("defaultPriorityHelp"))}
    ${!routes
      ? skeletonRows(2)
      : families.length
        ? families.map(
            ([family, list]) => html`<div class="priority-group">
              <small class="group-label">${family}</small>
              <ol class="priority-list">
                ${list.map(
                  (route, index) => html`<li class=${index === 0 ? "winner" : ""}>
                    <span class="rank">${index + 1}</span>
                    <div>
                      <strong>${route.interfaceAlias}</strong>
                      <small
                        >${route.onLink ? app.t("onLink") : route.nextHop} ·
                        ${app.tf("metricBreakdown", {
                          route: route.routeMetric ?? "—",
                          iface: route.interfaceMetric ?? "—",
                        })}</small
                      >
                    </div>
                    <span class="metric-value" title=${app.t("effectiveMetric")}
                      >${route.effectiveMetric ?? route.routeMetric ?? "—"}</span
                    >
                    ${index === 0 ? chip(app.t("preferred"), "accent") : nothing}
                  </li>`,
                )}
              </ol>
            </div>`,
          )
        : html`<p class="empty">${app.t("noDefaultRoutes")}</p>`}
  </section>`;
}

function renderDeviationTiles(app, adapters) {
  const routes = app.network.routes?.routes || [];
  const tiles = [
    {
      label: app.t("manualMetrics"),
      value: adapters.filter(hasManualMetric).length,
      icon: "settings",
      action: () => app.goToAdapters("manual"),
    },
    {
      label: app.t("staticRoutes"),
      value: routes.filter((route) => app.isStaticRoute(route)).length,
      icon: "routes",
      action: () => app.goToRoutes({ staticOnly: true }),
    },
    {
      label: app.t("disabledAdapters"),
      value: adapters.filter(isAdapterDisabled).length,
      icon: "power",
      action: () => app.goToAdapters("disabled"),
    },
  ];
  return html`<section class="card">
    ${cardHeader(app.t("changes"), app.t("changesHelp"))}
    <div class="tile-grid">
      ${tiles.map(
        (tile) => html`<button
          class="tile ${tile.value ? "attention" : ""}"
          @click=${tile.action}
          title=${app.t("showDetails")}
        >
          ${icon(tile.icon)}
          <strong>${tile.value}</strong>
          <span>${tile.label}</span>
        </button>`,
      )}
    </div>
  </section>`;
}

function renderAdapterList(app, adapters) {
  return html`<section class="card">
    ${cardHeader(
      app.t("adapters"),
      "",
      html`<button class="subtle" @click=${() => app.setView("adapters")}>
        ${app.t("viewAll")}${icon("chevronRight")}
      </button>`,
    )}
    ${!app.network.overview
      ? skeletonRows(3)
      : adapters.length
        ? html`<div class="row-list">
            ${sortAdapters(adapters).map(
              (item) => html`<button class="list-button" @click=${() => app.goToAdapter(item)}>
                <span class="status-dot ${isAdapterOnline(item) ? "online" : isAdapterDisabled(item) ? "off" : ""}"></span>
                ${icon(adapterIconName(item))}
                <span class="list-main">
                  <strong>${item.alias}</strong>
                  <small>${item.wifi?.ssid || primaryIpv4(item) || item.description || item.type}</small>
                </span>
                <span class="list-meta">${app.connectionLabel(item)}</span>
                ${icon("chevronRight", "muted")}
              </button>`,
            )}
          </div>`
        : html`<p class="empty">${app.t("noAdapters")}</p>`}
  </section>`;
}

function renderAutoConnect(app) {
  const rules = app.wifiAuto.rules || [];
  return html`<section class="card">
    ${cardHeader(
      app.t("wifiAutoConnect"),
      rules.length ? "" : app.t("autoNoneHelp"),
      html`<button class="subtle" @click=${() => app.goToAdapters("wifi")}>
        ${app.t("manage")}${icon("chevronRight")}
      </button>`,
    )}
    ${rules.length
      ? html`<div class="row-list">
          ${rules.map((rule) => {
            const status = app.wifiAutoStatus(rule.adapterGuid);
            const adapter = app.wifiAutoAdapter(rule.adapterGuid);
            const state = status?.state || (rule.enabled ? "waiting" : "disabled");
            return html`<div class="list-row">
              ${icon("wifi")}
              <span class="list-main">
                <strong>${rule.profileName}</strong>
                <small
                  >${adapter?.interfaceName || app.t("wifiAutoConfiguredAdapter")} ·
                  ${app.tf("everySeconds", { seconds: rule.intervalSeconds })}${rule.enabled &&
                  status?.nextCheck
                    ? ` · ${app.t("wifiAutoNextCheck")} ${formatTime(status.nextCheck, app.settings.language)}`
                    : ""}</small
                >
              </span>
              ${pill(app.wifiAutoStateClass(state), app.wifiAutoStateText(state))}
            </div>`;
          })}
        </div>`
      : nothing}
  </section>`;
}

function renderCoexistence(app) {
  const info = app.coexistence;
  const software = info?.software || [];
  const tunnels = info?.tunnels || [];
  const proxy = info?.systemProxy;
  if (!software.length && !tunnels.length && !proxy?.enabled) return nothing;
  const via = info.defaultVia;
  return html`<section class="card">
    ${cardHeader(app.t("coexistence"), app.t("coexistenceHelp"))}
    <div class="row-list">
      ${software.map(
        (item) => html`<div class="list-row">
          ${icon("shield")}
          <span class="list-main">
            <strong>${item.product}</strong>
            <small>${(item.processes || []).join(", ")}</small>
          </span>
        </div>`,
      )}
      <div class="list-row">
        ${icon("globe")}
        <span class="list-main">
          <strong>${app.t("systemProxy")}</strong>
          <small>${proxy?.enabled ? app.tf("proxyOn", { server: proxy.server || proxy.autoConfigUrl || "—" }) : app.t("proxyOff")}</small>
        </span>
        <span class="list-meta"
          >${via
            ? app.tf("trafficVia", { alias: via.alias, owner: app.ownerLabel(via.owner) })
            : app.t("trafficDirect")}</span
        >
      </div>
      ${tunnels.map(
        (tunnel) => html`<div class="list-row">
          ${icon("tunnel")}
          <span class="list-main">
            <strong>${tunnel.alias}</strong>
            <small>${app.managedLabel(tunnel.owner)}${tunnel.ipv4 ? ` · ${tunnel.ipv4}` : ""}</small>
          </span>
          ${pill(tunnel.up ? "success" : "neutral", tunnel.up ? app.t("connected") : app.t("disconnected"))}
        </div>`,
      )}
    </div>
  </section>`;
}

export function renderOverview(app) {
  const adapters = app.interfaces();
  const routes = app.network.routes;
  const preferred = sortDefaultCandidates(
    routes?.defaultCandidates || (routes?.routes || []).filter((route) => route.defaultRoute),
  )[0];
  const usable = adapters.filter((item) => item.defaultRoute && !isAdapterDisabled(item));
  // The active path is the adapter that owns the preferred default route, or
  // the tunnel (v2rayN TUN, EasyTier exit node) that captures default traffic.
  const via = app.coexistence?.defaultVia;
  const active =
    (via && adapters.find((item) => item.interfaceIndex === via.interfaceIndex)) ||
    usable.find((item) => item.interfaceIndex === preferred?.interfaceIndex) ||
    sortAdapters(usable)[0];
  return html`<div class="dashboard">
    <div class="span-2">${renderHero(app, active)}</div>
    ${renderDefaultRoutes(app)} ${renderDeviationTiles(app, adapters)}
    ${renderAdapterList(app, adapters)} ${renderAutoConnect(app)} ${renderCoexistence(app)}
  </div>`;
}
