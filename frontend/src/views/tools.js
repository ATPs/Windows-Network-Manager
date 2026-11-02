import { html, nothing } from "lit";
import { icon } from "../icons.js";
import { formatTime } from "../format.js";
import { buttonContent, cardHeader, chip, emptyState, fact, pill, segmented, signalBars } from "../ui.js";

const MAX_CONNECTION_ROWS = 500;

/** Access points per channel, grouped by band, busiest first. */
export function channelSummary(accessPoints = []) {
  const bands = new Map();
  for (const ap of accessPoints) {
    if (!ap.band || ap.channel == null) continue;
    const channels = bands.get(ap.band) || new Map();
    channels.set(ap.channel, (channels.get(ap.channel) || 0) + 1);
    bands.set(ap.band, channels);
  }
  return [...bands.entries()]
    .sort(([a], [b]) => a.localeCompare(b, undefined, { numeric: true }))
    .map(([band, channels]) => ({
      band,
      channels: [...channels.entries()]
        .map(([channel, count]) => ({ channel, count }))
        .sort((a, b) => b.count - a.count || a.channel - b.channel),
    }));
}

/** Filters the connection table by text, protocol and state. */
export function filterConnections(list = [], { search = "", protocol = "all", state = "all" } = {}) {
  const needle = search.trim().toLowerCase();
  return list.filter((row) => {
    if (protocol !== "all" && row.protocol !== protocol) return false;
    if (state === "listening" && !(row.state === "Listen" || row.protocol === "UDP")) return false;
    if (state === "established" && row.state !== "Established") return false;
    if (!needle) return true;
    return [row.localAddress, row.localPort, row.remoteAddress, row.remotePort, row.process, row.pid, row.adapter, row.product]
      .some((value) => value != null && String(value).toLowerCase().includes(needle));
  });
}

const endpoint = (address, port) =>
  address == null ? "—" : address.includes(":") ? `[${address}]:${port}` : `${address}:${port}`;

function echoText(app, result) {
  if (!result?.ok) return app.t("noReply");
  return app.tf("echoResult", { avg: result.avgMs ?? "—", loss: result.lossPercent ?? 0 });
}

function verdictPill(app, verdict) {
  const map = {
    ok: ["success", "verdictOk", "success"],
    gateway: ["danger", "verdictGateway", "error"],
    internet: ["warning", "verdictInternet", "warning"],
    dns: ["warning", "verdictDns", "warning"],
  };
  const [kind, key, iconName] = map[verdict] || map.ok;
  return pill(kind, app.t(key), iconName);
}

function viaText(app, via) {
  return via ? app.tf("defaultPathVia", { alias: via.alias, owner: app.ownerLabel(via.owner) }) : app.t("defaultPathDirect");
}

function renderConnectivity(app) {
  const result = app.tools.connectivity;
  const busy = app.isBusy("tools:connectivity");
  const proxy = app.coexistence?.systemProxy;
  const kindLabel = { gateway: "checkGateway", dns: "checkDns", internet: "checkInternet" };
  const dns = result?.defaultPath?.dns;
  return html`<section class="card span-2">
    ${cardHeader(
      app.t("connectivityTest"),
      app.t("connectivityHelp"),
      html`<button class="accent" ?disabled=${busy} @click=${() => app.runConnectivityTest()}>
        ${buttonContent(busy, "diagnostics", app.t("runTest"))}
      </button>`,
    )}
    ${proxy?.enabled ? html`<p class="help-text">${icon("info")} ${app.tf("proxyNote", { server: proxy.server || "—" })}</p>` : nothing}
    ${result
      ? html`<div class="tool-results">
          <div class="tool-block">
            <div class="tool-block-head">
              ${icon("globe")}<strong>${app.t("defaultPath")}</strong>
              <span class="muted">${viaText(app, result.defaultPath?.via)}</span>
            </div>
            <table class="compact-table">
              <tbody>
                <tr>
                  <td>${app.t("checkInternet")}</td>
                  <td class="mono">${result.defaultPath?.internet?.target || "1.1.1.1"}</td>
                  <td>${echoText(app, result.defaultPath?.internet)}</td>
                </tr>
                <tr>
                  <td>${app.t("dnsLookup")}</td>
                  <td class="mono">${dns?.host || "—"}</td>
                  <td>
                    ${dns?.ok ? (dns.addresses || []).slice(0, 2).join(", ") : dns?.error || app.t("noReply")}
                    ${dns?.fakeIp ? chip(app.t("fakeIpDns"), "warning") : nothing}
                  </td>
                </tr>
              </tbody>
            </table>
          </div>
          ${(result.adapters || []).length
            ? result.adapters.map(
                (adapter) => html`<div class="tool-block">
                  <div class="tool-block-head">
                    <strong>${adapter.alias}</strong>
                    <span class="muted mono">${adapter.source || ""}</span>
                    ${adapter.managedBy ? chip(app.managedLabel(adapter.managedBy), "accent") : nothing}
                    <span class="toolbar-spacer"></span>
                    ${verdictPill(app, adapter.verdict)}
                  </div>
                  <table class="compact-table">
                    <tbody>
                      ${(adapter.checks || []).map(
                        (check) => html`<tr class=${check.ok ? "" : "failed"}>
                          <td>${app.t(kindLabel[check.kind] || check.kind)}</td>
                          <td class="mono">${check.target}</td>
                          <td>${echoText(app, check)}</td>
                        </tr>`,
                      )}
                    </tbody>
                  </table>
                </div>`,
              )
            : html`<p class="empty">${app.t("noTestableAdapters")}</p>`}
          <small class="muted">${formatTime(result.testedAt, app.settings.language)}</small>
        </div>`
      : nothing}
  </section>`;
}

function renderTrace(app) {
  const trace = app.tools.trace;
  if (!trace) return nothing;
  const status = !trace.done
    ? app.tf("traceRunning", { target: trace.target, address: trace.address })
    : trace.error || (trace.reached ? app.t("traceReached") : trace.cancelled ? app.t("traceCancelled") : app.tf("traceIncomplete", { hops: trace.maxHops }));
  return html`<div class="tool-block">
    <div class="tool-block-head">${icon("routes")}<strong>${app.t("trace")}</strong><span class="muted">${status}</span></div>
    <table class="compact-table">
      <thead>
        <tr>
          <th class="num">${app.t("hop")}</th>
          <th>${app.t("resolvedAddress")}</th>
          <th>${app.t("latency")}</th>
        </tr>
      </thead>
      <tbody>
        ${trace.hops.map(
          (hop) => html`<tr class=${hop.status === "reached" ? "reached" : ""}>
            <td class="num">${hop.hop}</td>
            <td class="mono">${hop.address || "*"}</td>
            <td>${hop.rttsMs.map((rtt) => (rtt == null ? "*" : `${rtt} ms`)).join("  ")}</td>
          </tr>`,
        )}
      </tbody>
    </table>
  </div>`;
}

function renderRouteLookup(app) {
  const result = app.tools.lookup;
  const lookupBusy = app.isBusy("tools:lookup");
  const traceBusy = app.isBusy("tools:trace") || app.isBusy("tools:trace-start");
  const empty = !app.tools.lookupHost.trim();
  const route = result?.route;
  return html`<section class="card">
    ${cardHeader(app.t("routeLookup"), app.t("routeLookupHelp"))}
    <form
      class="tool-form"
      @submit=${(event) => {
        event.preventDefault();
        app.lookupRoute();
      }}
    >
      <input
        type="text"
        maxlength="253"
        spellcheck="false"
        placeholder=${app.t("hostPlaceholder")}
        aria-label=${app.t("hostPlaceholder")}
        .value=${app.tools.lookupHost}
        @input=${(event) => {
          app.tools.lookupHost = event.target.value;
          app.requestUpdate();
        }}
      />
      <button class="accent" type="submit" ?disabled=${empty || lookupBusy}>
        ${buttonContent(lookupBusy, "search", app.t("lookUp"))}
      </button>
      ${app.isBusy("tools:trace")
        ? html`<button class="standard" type="button" @click=${() => app.cancelTrace()}>${icon("close")}${app.t("stopTrace")}</button>`
        : html`<button class="standard" type="button" ?disabled=${empty || traceBusy} @click=${() => app.startTrace()}>
            ${buttonContent(app.isBusy("tools:trace-start"), "routes", app.t("trace"))}
          </button>`}
    </form>
    ${result
      ? html`<div class="fact-grid two">
          ${fact(app.t("resolvedAddress"), result.address, { copy: result.address, app })}
          ${fact(app.t("interface"), route ? `${route.interfaceAlias} #${route.interfaceIndex}` : null)}
          ${fact(app.t("matchedRoute"), route?.destinationPrefix)}
          ${fact(app.t("nextHop"), route?.nextHop)}
          ${fact(app.t("sourceAddress"), route?.sourceAddress)}
          ${fact(app.t("effectiveMetric"), route?.effectiveMetric ?? route?.routeMetric)}
        </div>
        <div class="chips">
          ${result.managedBy ? chip(app.managedLabel(result.managedBy), "accent") : nothing}
          ${result.fakeIp ? chip(app.t("fakeIpDns"), "warning") : nothing}
        </div>`
      : nothing}
    ${renderTrace(app)}
  </section>`;
}

function renderWifiScan(app) {
  const adapters = app.wifiAdapters();
  const scan = app.tools.wifiScan;
  const busy = app.isBusy("tools:wifi");
  const selected = app.tools.wifiAdapter || adapters[0]?.guid || "";
  const points = [...(scan?.accessPoints || [])].sort((a, b) => b.signalPercent - a.signalPercent);
  const summary = channelSummary(points);
  return html`<section class="card">
    ${cardHeader(
      app.t("nearbyWifi"),
      app.t("nearbyWifiHelp"),
      adapters.length
        ? html`${adapters.length > 1
              ? html`<select
                  aria-label=${app.t("adapters")}
                  .value=${selected}
                  @change=${(event) => {
                    app.tools.wifiAdapter = event.target.value;
                    app.requestUpdate();
                  }}
                >
                  ${adapters.map((item) => html`<option value=${item.guid}>${item.alias}</option>`)}
                </select>`
              : nothing}
            <button class="accent" ?disabled=${busy || !selected} @click=${() => app.scanWifi(selected)}>
              ${buttonContent(busy, "wifi", app.t("scan"))}
            </button>`
        : nothing,
    )}
    ${!adapters.length ? html`<p class="empty">${app.t("noWifiAdapters")}</p>` : nothing}
    ${scan
      ? html`${scan.fresh ? nothing : html`<p class="help-text">${icon("info")} ${app.t("cachedScan")}</p>`}
          <div class="channel-summary">
            <small class="group-label">${app.t("channelUse")}</small>
            ${summary.map(
              (group) => html`<div class="channel-row">
                <strong>${group.band}</strong>
                ${group.channels.map(({ channel, count }) =>
                  chip(`${channel} × ${count}${count >= 4 ? ` · ${app.t("crowded")}` : ""}`, count >= 4 ? "warning" : ""),
                )}
              </div>`,
            )}
          </div>
          <div class="table-scroll">
            <table class="compact-table">
              <thead>
                <tr>
                  <th>SSID</th>
                  <th>${app.t("signal")}</th>
                  <th>${app.t("channel")}</th>
                  <th>${app.t("security")}</th>
                  <th>BSSID</th>
                  <th><span class="visually-hidden">${app.t("actions")}</span></th>
                </tr>
              </thead>
              <tbody>
                ${points.map(
                  (ap) => html`<tr>
                    <td>
                      ${ap.ssid || html`<span class="muted">${app.t("hiddenNetwork")}</span>`}
                      ${ap.connected ? chip(app.t("connected"), "accent") : ap.savedProfile ? chip(app.t("savedProfile")) : nothing}
                    </td>
                    <td><span class="fact-value">${signalBars(ap.signalPercent, `${ap.signalPercent}%`)}<span>${ap.signalPercent}%</span></span></td>
                    <td>${ap.band ? `${ap.band} · ${ap.channel}` : `${ap.frequencyMhz} MHz`}</td>
                    <td>${ap.security || "—"}</td>
                    <td class="mono muted">${ap.bssid}</td>
                    <td>
                      ${app.canConnectNew(ap)
                        ? html`<button
                            class="subtle compact"
                            ?disabled=${app.isBusyPrefix("wifi-new")}
                            @click=${() => app.startConnectNew(ap)}
                          >
                            ${buttonContent(app.isBusy(`wifi-new:preview:${ap.ssid}`), "wifi", app.t("connectEllipsis"))}
                          </button>`
                        : nothing}
                    </td>
                  </tr>`,
                )}
              </tbody>
            </table>
          </div>`
      : nothing}
  </section>`;
}

function renderConnections(app) {
  const data = app.tools.connections;
  const busy = app.isBusy("tools:connections");
  const filters = app.tools.connectionFilters;
  const rows = filterConnections(data?.connections || [], filters);
  const shown = rows.slice(0, MAX_CONNECTION_ROWS);
  return html`<section class="card span-2">
    ${cardHeader(
      app.t("activeConnections"),
      app.t("activeConnectionsHelp"),
      html`<button class="standard" ?disabled=${busy} @click=${() => app.loadConnections()}>
        ${buttonContent(busy, "refresh", data ? app.t("reload") : app.t("loadConnections"))}
      </button>`,
    )}
    ${data
      ? html`<div class="toolbar">
            <label class="search-box"
              >${icon("search")}<input
                type="search"
                placeholder=${app.t("connectionSearch")}
                aria-label=${app.t("connectionSearch")}
                .value=${filters.search}
                @input=${(event) => app.setConnectionFilter({ search: event.target.value })}
            /></label>
            ${segmented(
              [
                ["all", app.t("filterAll")],
                ["TCP", "TCP"],
                ["UDP", "UDP"],
              ],
              filters.protocol,
              (protocol) => app.setConnectionFilter({ protocol }),
              app.t("protocol"),
            )}
            ${segmented(
              [
                ["all", app.t("filterAll")],
                ["established", app.t("stateEstablished")],
                ["listening", app.t("stateListening")],
              ],
              filters.state,
              (state) => app.setConnectionFilter({ state }),
              app.t("connectionState"),
            )}
            <span class="toolbar-spacer"></span>
            <span class="muted">${app.tf("showingCount", { shown: shown.length, total: rows.length })}</span>
          </div>
          <div class="table-scroll tall">
            <table class="compact-table">
              <thead>
                <tr>
                  <th>${app.t("protocol")}</th>
                  <th>${app.t("localEndpoint")}</th>
                  <th>${app.t("remoteEndpoint")}</th>
                  <th>${app.t("connectionState")}</th>
                  <th>${app.t("process")}</th>
                  <th>${app.t("interface")}</th>
                </tr>
              </thead>
              <tbody>
                ${shown.map(
                  (row) => html`<tr>
                    <td>${row.protocol}<span class="muted"> ${row.family === "IPv6" ? "v6" : ""}</span></td>
                    <td class="mono">${endpoint(row.localAddress, row.localPort)}</td>
                    <td class="mono">${row.remoteAddress == null ? "—" : endpoint(row.remoteAddress, row.remotePort)}</td>
                    <td>${row.state || "—"}</td>
                    <td>
                      ${row.process || "—"} <span class="muted">${row.pid}</span>
                      ${row.product ? chip(row.product, "accent") : nothing}
                    </td>
                    <td>${row.adapter || "—"}</td>
                  </tr>`,
                )}
              </tbody>
            </table>
          </div>`
      : nothing}
  </section>`;
}

export function renderTools(app) {
  if (!app.network.overview)
    return html`<section class="card">${emptyState("tools", app.t("tools"), app.t("toolsSubtitle"))}</section>`;
  return html`<div class="dashboard">
    ${renderConnectivity(app)} ${renderRouteLookup(app)} ${renderWifiScan(app)} ${renderConnections(app)}
  </div>`;
}
