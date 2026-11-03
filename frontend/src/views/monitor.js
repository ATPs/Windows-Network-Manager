import { html, nothing } from "lit";
import { icon } from "../icons.js";
import { formatBytes, formatMs, formatTime, summarizeLatency, summarizeRates } from "../format.js";
import { barChart, lineChart } from "../charts.js";
import { renderNotificationSettings } from "./settings.js";
import { buttonContent, cardHeader, chip, emptyState, segmented, skeletonRows, switchInput } from "../ui.js";

const EVENT_ICONS = { adapter: "adapters", wifi: "wifi", defaultRoute: "routes", trigger: "profiles" };
const CARD_KEYS = ["traffic", "latency", "signal", "daily", "events", "notifications"];

/** Events matching the kind filter; tunnel up/down events only on request. */
export function filterEvents(events = [], { kind = "all", tunnels = false } = {}) {
  return events.filter((event) => (kind === "all" || event.kind === kind) && (tunnels || !event.tunnel));
}

const clock = (language) => (time) =>
  new Intl.DateTimeFormat(language, { hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(new Date(time));
const dateClock = (language) => (time) =>
  new Intl.DateTimeFormat(language, { month: "numeric", day: "numeric", hour: "2-digit", minute: "2-digit" }).format(new Date(time));

/** Interfaces the picker offers: live ones plus any seen in saved history. */
export function monitorInterfaces(snapshot, history) {
  const seen = new Map();
  for (const item of snapshot?.traffic?.interfaces || []) seen.set(item.key, { key: item.key, alias: item.alias, owner: item.owner, hardware: item.hardware, up: item.up });
  for (const item of history?.available || []) if (!seen.has(item.key)) seen.set(item.key, { ...item, up: false });
  return [...seen.values()];
}

/** Numbers for the stat tiles, from live samples or saved history. */
export function monitorStats(app) {
  const { range, snapshot, history, events } = app.monitor;
  const latencyBy = (kind, list, samplesOf) => {
    const values = list.filter((item) => item.kind === kind).flatMap(samplesOf);
    return summarizeLatency(values);
  };
  if (range === "live") {
    const total = snapshot?.traffic?.total || [];
    const rx = summarizeRates(total.map(([time, value]) => [time, value]));
    const tx = summarizeRates(total.map(([time, , value]) => [time, value]));
    const latest = total[total.length - 1];
    const today = new Date().toDateString();
    const disconnects = (events || []).filter(
      (event) => event.code === "event.adapterDown" && !event.tunnel && new Date(event.time).toDateString() === today,
    ).length;
    const latency = snapshot?.latency || [];
    return {
      currentRx: latest?.[1] ?? null,
      currentTx: latest?.[2] ?? null,
      avgRx: rx.avg,
      avgTx: tx.avg,
      peakRx: rx.peak,
      peakTx: tx.peak,
      totalRx: rx.total,
      totalTx: tx.total,
      gateway: latencyBy("gateway", latency, (item) => item.samples.map(([, rtt]) => rtt)),
      dns: latencyBy("dns", latency, (item) => item.samples.map(([, rtt]) => rtt)),
      disconnects,
      routeChanges: null,
    };
  }
  const total = history?.total || {};
  const latency = history?.latency || [];
  const merge = (kind) => {
    const items = latency.filter((item) => item.kind === kind).map((item) => item.stats);
    if (!items.length) return summarizeLatency([]);
    const sent = items.reduce((sum, item) => sum + item.sent, 0);
    const received = items.reduce((sum, item) => sum + item.received, 0);
    const avg = received ? items.reduce((sum, item) => sum + (item.avg ?? 0) * item.received, 0) / received : null;
    return { sent, avg, p95: Math.max(...items.map((item) => item.p95 ?? 0)) || null, max: Math.max(...items.map((item) => item.max ?? 0)) || null, lossPct: sent ? Math.round(((sent - received) * 100) / sent) : null };
  };
  return {
    currentRx: null,
    currentTx: null,
    avgRx: total.rxAvg ?? null,
    avgTx: total.txAvg ?? null,
    peakRx: total.rxPeak ?? null,
    peakTx: total.txPeak ?? null,
    totalRx: total.rxBytes ?? 0,
    totalTx: total.txBytes ?? 0,
    gateway: merge("gateway"),
    dns: merge("dns"),
    disconnects: history?.events?.down ?? 0,
    routeChanges: history?.events?.route ?? 0,
  };
}

function renderToolbar(app) {
  const { range, selected, cards, rateUnit, snapshot, history } = app.monitor;
  const interfaces = monitorInterfaces(snapshot, history);
  return html`<section class="card span-2 monitor-toolbar">
    <div class="toolbar">
      ${segmented(
        [
          ["live", app.t("rangeLive")],
          ["1h", app.t("range1h")],
          ["24h", app.t("range24h")],
          ["7d", app.t("range7d")],
          ["30d", app.t("range30d")],
        ],
        range,
        (value) => app.setMonitorRange(value),
        app.t("timeRange"),
      )}
      <span class="toolbar-spacer"></span>
      ${segmented(
        [
          ["bits", "Mbps"],
          ["bytes", "MB/s"],
        ],
        rateUnit,
        (unit) => app.setRateUnit(unit),
        app.t("rateUnit"),
      )}
    </div>
    <p class="help-text">${range === "live" ? app.t("rangeLiveHelp") : app.tf("rangeHistoryHelp", { minutes: Math.round((history?.bucketSeconds || 60) / 60) })}</p>
    ${interfaces.length
      ? html`<div class="field">
          <span>${app.t("monitorInterfaces")}</span>
          <div class="chip-filters">
            ${interfaces.map(
              (item) => html`<button
                class="filter-chip ${selected.includes(item.key) ? "selected" : ""}"
                aria-pressed=${String(selected.includes(item.key))}
                @click=${() => app.toggleMonitorInterface(item.key)}
              >
                ${item.alias}${item.owner ? html` <span class="count">${app.managedLabel(item.owner)}</span>` : nothing}
              </button>`,
            )}
          </div>
          <small class="field-help">${app.t("monitorInterfacesHelp")}</small>
        </div>`
      : nothing}
    <details class="monitor-customize">
      <summary>${app.t("customizeMonitor")}</summary>
      <div class="check-row">
        ${CARD_KEYS.map(
          (key) => html`<label class="check"
            ><input type="checkbox" .checked=${cards[key] !== false} @change=${(event) => app.setMonitorCard(key, event.target.checked)} /><span
              >${app.t(`monitorCard_${key}`)}</span
            ></label
          >`,
        )}
      </div>
    </details>
  </section>`;
}

function tile(label, value, detail = "") {
  return html`<div class="stat-tile">
    <small>${label}</small>
    <strong>${value}</strong>
    ${detail ? html`<span>${detail}</span>` : nothing}
  </div>`;
}

function renderStats(app) {
  const stats = monitorStats(app);
  const rate = (value) => (value == null ? "—" : app.formatRate(value));
  const live = app.monitor.range === "live";
  const latencyDetail = (item) =>
    item.sent ? app.tf("latencyDetail", { p95: formatMs(item.p95), loss: item.lossPct ?? 0 }) : app.t("noLatencyData");
  return html`<section class="span-2 stat-tiles" aria-label=${app.t("monitorStatistics")}>
    ${live
      ? tile(app.t("statNow"), html`↓ ${rate(stats.currentRx)}<br />↑ ${rate(stats.currentTx)}`)
      : nothing}
    ${tile(app.t("statAverage"), html`↓ ${rate(stats.avgRx)}<br />↑ ${rate(stats.avgTx)}`)}
    ${tile(app.t("statPeak"), html`↓ ${rate(stats.peakRx)}<br />↑ ${rate(stats.peakTx)}`)}
    ${tile(app.t("statTransferred"), html`↓ ${formatBytes(stats.totalRx)}<br />↑ ${formatBytes(stats.totalTx)}`, live ? app.t("statLastMinutes") : "")}
    ${tile(app.t("statGatewayLatency"), formatMs(stats.gateway.avg), latencyDetail(stats.gateway))}
    ${tile(app.t("statDnsLatency"), formatMs(stats.dns.avg), latencyDetail(stats.dns))}
    ${tile(
      app.t("statDisconnects"),
      String(stats.disconnects ?? 0),
      live ? app.t("statToday") : app.tf("statRouteChanges", { count: stats.routeChanges ?? 0 }),
    )}
  </section>`;
}

function trafficSeries(app, points, rxIndex = 1, txIndex = 2) {
  return [
    { key: "rx", label: app.t("download"), tone: "series-1", points: points.map((point) => [point[0], point[rxIndex]]) },
    { key: "tx", label: app.t("upload"), tone: "series-2", points: points.map((point) => [point[0], point[txIndex]]) },
  ];
}

function renderTraffic(app) {
  const { range, snapshot, history, selected } = app.monitor;
  const live = range === "live";
  const language = app.settings.language;
  const formatTimeFn = live ? clock(language) : dateClock(language);
  const totalPoints = live ? snapshot?.traffic?.total || [] : history?.total?.points || [];
  const chart = (id, points, height = 160) =>
    lineChart({
      id,
      series: trafficSeries(app, points),
      height,
      formatValue: (value) => app.formatRate(value),
      formatTime: formatTimeFn,
      hover: app.monitor.hover,
      onHover: (hover) => app.setMonitorHover(hover),
      ariaLabel: app.t("trafficChartLabel"),
    });
  const loading = live ? !snapshot : !history;
  const totalChart = loading ? nothing : chart("traffic", totalPoints);
  const liveInterfaces = snapshot?.traffic?.interfaces || [];
  const interfaceRows = live ? liveInterfaces : history?.interfaces || [];
  const perInterface = selected
    .map((key) => {
      if (live) {
        const item = liveInterfaces.find((entry) => entry.key === key);
        return item && { key, alias: item.alias, owner: item.owner, points: item.samples || [] };
      }
      const item = (history?.interfaces || []).find((entry) => entry.key === key);
      return item && { key, alias: item.alias, owner: item.owner, points: item.points || [] };
    })
    .filter(Boolean);
  return html`<section class="card span-2">
    ${cardHeader(live ? app.t("liveTraffic") : app.t("trafficHistory"), live ? app.t("liveTrafficHelp") : app.t("trafficHistoryHelp"))}
    ${loading
      ? skeletonRows(3)
      : totalChart === nothing
        ? html`<p class="empty">${live ? app.t("collectingSamples") : app.t("noHistoryYet")}</p>`
        : html`<h3 class="chart-title">${app.t("totalPhysical")}</h3>${totalChart}`}
    ${perInterface.map((item) => {
      const small = chart(`traffic-${item.key}`, item.points, 120);
      return html`<div class="tool-block">
        <div class="tool-block-head">${icon("adapters")}<strong>${item.alias}</strong>${item.owner ? chip(app.managedLabel(item.owner), "accent") : nothing}</div>
        ${small === nothing ? html`<p class="empty">${app.t("collectingSamples")}</p>` : small}
      </div>`;
    })}
    ${interfaceRows.length
      ? html`<div class="table-scroll"><table class="compact-table">
          <thead>
            <tr>
              <th>${app.t("interface")}</th>
              ${live
                ? html`<th class="num">↓ ${app.t("download")}</th>
                    <th class="num">↑ ${app.t("upload")}</th>`
                : html`<th class="num">↓ ${app.t("statTransferred")}</th>
                    <th class="num">↑ ${app.t("statTransferred")}</th>
                    <th class="num">${app.t("statPeak")} ↓</th>
                    <th class="num">${app.t("uptime")}</th>`}
            </tr>
          </thead>
          <tbody>
            ${interfaceRows.map(
              (item) => html`<tr class=${live && !item.up ? "muted" : ""}>
                <td>
                  ${item.alias}
                  ${item.owner ? chip(app.managedLabel(item.owner), "accent") : nothing}
                  ${live && !item.up ? chip(app.t("disconnected"), "muted") : nothing}
                </td>
                ${live
                  ? html`<td class="num">${app.formatRate(item.rxBps)}</td>
                      <td class="num">${app.formatRate(item.txBps)}</td>`
                  : html`<td class="num">${formatBytes(item.rxBytes)}</td>
                      <td class="num">${formatBytes(item.txBytes)}</td>
                      <td class="num">${app.formatRate(item.rxPeak)}</td>
                      <td class="num">${item.uptimePct == null ? "—" : `${item.uptimePct}%`}</td>`}
              </tr>`,
            )}
          </tbody>
        </table></div>
        <p class="help-text">${icon("info")} ${app.t("tunnelTrafficNote")}</p>`
      : nothing}
  </section>`;
}

function renderLatency(app) {
  const { range, snapshot, history } = app.monitor;
  const live = range === "live";
  const enabled = snapshot?.latencyEnabled !== false;
  const targets = live
    ? (snapshot?.latency || []).map((item) => ({
        ...item,
        points: item.samples.map(([time, rtt]) => [time, rtt]),
        stats: summarizeLatency(item.samples.map(([, rtt]) => rtt)),
      }))
    : (history?.latency || []).map((item) => ({ ...item, points: item.points.map(([time, avg]) => [time, avg]) }));
  const language = app.settings.language;
  return html`<section class="card">
    ${cardHeader(app.t("latencyTitle"), app.t("latencyHelp"))}
    ${!enabled
      ? html`<p class="empty">${app.t("latencyOff")}</p>`
      : (live ? !snapshot : !history)
        ? skeletonRows(2)
        : targets.length
          ? targets.map((target) => {
              const chart = lineChart({
                id: `latency-${target.kind}-${target.address}`,
                series: [{ key: "rtt", label: app.t("roundTrip"), tone: "series-1", points: target.points }],
                height: 110,
                area: false,
                formatValue: (value) => formatMs(value),
                formatTime: live ? clock(language) : dateClock(language),
                hover: app.monitor.hover,
                onHover: (hover) => app.setMonitorHover(hover),
                ariaLabel: `${app.t("latencyTitle")} · ${target.address}`,
              });
              return html`<div class="tool-block">
                <div class="tool-block-head">
                  ${icon(target.kind === "gateway" ? "routes" : "globe")}<strong>${app.t(target.kind === "gateway" ? "checkGateway" : "checkDns")}</strong>
                  <span class="muted mono">${target.address}</span>
                  <span class="toolbar-spacer"></span>
                  <span>${formatMs(target.stats.avg)} · ${app.tf("lossShort", { loss: target.stats.lossPct ?? 0 })}</span>
                </div>
                ${chart === nothing ? html`<p class="empty">${app.t("collectingSamples")}</p>` : chart}
                ${target.stats.lossPct === 100 ? html`<p class="help-text">${icon("info")} ${app.t("noReplyHint")}</p>` : nothing}
              </div>`;
            })
          : html`<p class="empty">${app.t("noLatencyTargets")}</p>`}
  </section>`;
}

function renderSignal(app) {
  const { range, snapshot, history } = app.monitor;
  const live = range === "live";
  const language = app.settings.language;
  const adapters = live
    ? (snapshot?.signal || []).map((adapter) => ({ ...adapter, points: (adapter.samples || []).map(([time, percent]) => [time, percent]) }))
    : (history?.wifi || []).map((adapter) => ({ ...adapter, ssid: (adapter.ssids || []).join(", "), points: adapter.points.map(([time, avg]) => [time, avg]) }));
  return html`<section class="card">
    ${cardHeader(app.t("signalHistory"), live ? app.t("signalHistoryHelp") : app.t("signalSavedHelp"))}
    ${(live ? !snapshot : !history)
      ? skeletonRows(2)
      : adapters.length
        ? adapters.map((adapter) => {
            const chart = lineChart({
              id: `signal-${adapter.adapterGuid}`,
              series: [{ key: "signal", label: app.t("signal"), tone: "series-1", points: adapter.points }],
              max: 100,
              height: 120,
              area: false,
              formatValue: (value) => `${Math.round(value)}%`,
              formatTime: live ? clock(language) : dateClock(language),
              hover: app.monitor.hover,
              onHover: (hover) => app.setMonitorHover(hover),
              ariaLabel: `${app.t("signalHistory")} · ${adapter.alias}`,
            });
            return html`<div class="tool-block">
              <div class="tool-block-head">
                ${icon("wifi")}<strong>${adapter.alias}</strong>
                <span class="muted">${adapter.ssid || app.t("disconnected")}</span>
                <span class="toolbar-spacer"></span>
                <span>${live && adapter.signalPercent != null ? `${adapter.signalPercent}%` : ""}</span>
              </div>
              ${chart === nothing ? html`<p class="empty">${app.t("collectingSamples")}</p>` : chart}
            </div>`;
          })
        : html`<p class="empty">${app.t("noSignalHistory")}</p>`}
  </section>`;
}

function renderDaily(app) {
  const history = app.monitor.history;
  const days = history?.daily || [];
  return html`<section class="card span-2">
    ${cardHeader(app.t("dailyUsage"), app.t("dailyUsageHelp"))}
    ${!history
      ? skeletonRows(2)
      : days.length
        ? html`${barChart({
              items: days.map((day) => ({ label: day.day.slice(5), values: [day.rxBytes, day.txBytes] })),
              formatValue: formatBytes,
              ariaLabel: app.t("dailyUsage"),
              legend: [app.t("download"), app.t("upload")],
            })}
            <div class="table-scroll">
              <table class="compact-table">
                <thead>
                  <tr>
                    <th>${app.t("day")}</th>
                    <th class="num">↓ ${app.t("download")}</th>
                    <th class="num">↑ ${app.t("upload")}</th>
                    <th class="num">${app.t("statDisconnects")}</th>
                    <th class="num">${app.t("routeChangesShort")}</th>
                    <th class="num">${app.t("wifiChangesShort")}</th>
                  </tr>
                </thead>
                <tbody>
                  ${[...days].reverse().map(
                    (day) => html`<tr>
                      <td>${day.day}</td>
                      <td class="num">${formatBytes(day.rxBytes)}</td>
                      <td class="num">${formatBytes(day.txBytes)}</td>
                      <td class="num">${day.disconnects}</td>
                      <td class="num">${day.routeChanges}</td>
                      <td class="num">${day.wifiChanges}</td>
                    </tr>`,
                  )}
                </tbody>
              </table>
            </div>`
        : html`<p class="empty">${app.t("noHistoryYet")}</p>`}
  </section>`;
}

function renderEvents(app) {
  const events = app.monitor.events;
  const filters = app.monitor.eventFilters;
  const rows = filterEvents(events || [], filters);
  const busy = app.isBusy("monitor:clear");
  return html`<section class="card span-2">
    ${cardHeader(
      app.t("eventHistory"),
      app.t("eventHistoryHelp"),
      html`<button class="subtle" ?disabled=${!events?.length || busy} @click=${() => app.clearNetworkEvents()}>
        ${buttonContent(busy, "trash", app.t("clearHistory"))}
      </button>`,
    )}
    <div class="toolbar">
      ${segmented(
        [
          ["all", app.t("filterAll")],
          ["adapter", app.t("adapters")],
          ["wifi", "Wi-Fi"],
          ["defaultRoute", app.t("defaultRoute")],
        ],
        filters.kind,
        (kind) => app.setEventFilter({ kind }),
        app.t("filter"),
      )}
      ${switchInput(filters.tunnels, app.t("showTunnelEvents"), (tunnels) => app.setEventFilter({ tunnels }))}
    </div>
    ${events == null
      ? skeletonRows(3)
      : rows.length
        ? html`<ol class="event-list">
            ${rows.map(
              (event) => html`<li>
                ${icon(EVENT_ICONS[event.kind] || "info")}
                <span>${app.msg(event)}</span>
                ${event.tunnel ? chip(app.t("tunnelEvent"), "muted") : nothing}
                <small class="muted">${formatTime(event.time, app.settings.language)}</small>
              </li>`,
            )}
          </ol>`
        : emptyState("clock", app.t("noEvents"), app.t("noEventsHelp"))}
  </section>`;
}

export function renderMonitor(app) {
  const cards = app.monitor.cards;
  const show = (key) => cards[key] !== false;
  const live = app.monitor.range === "live";
  return html`<div class="dashboard">
    ${renderToolbar(app)} ${renderStats(app)} ${show("traffic") ? renderTraffic(app) : nothing}
    ${show("latency") ? renderLatency(app) : nothing} ${show("signal") ? renderSignal(app) : nothing}
    ${show("daily") && !live ? renderDaily(app) : nothing} ${show("notifications") ? renderNotificationSettings(app) : nothing}
    ${show("events") ? renderEvents(app) : nothing}
  </div>`;
}
