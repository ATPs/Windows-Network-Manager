import { html, nothing } from "lit";
import { icon } from "../icons.js";
import { formatTime } from "../format.js";
import { formatRate, lineChart } from "../charts.js";
import { renderNotificationSettings } from "./settings.js";
import { buttonContent, cardHeader, chip, emptyState, segmented, skeletonRows, switchInput } from "../ui.js";

const EVENT_ICONS = { adapter: "adapters", wifi: "wifi", defaultRoute: "routes", trigger: "profiles" };

/** Events matching the kind filter; tunnel up/down events only on request. */
export function filterEvents(events = [], { kind = "all", tunnels = false } = {}) {
  return events.filter((event) => (kind === "all" || event.kind === kind) && (tunnels || !event.tunnel));
}

const clock = (language) => (time) =>
  new Intl.DateTimeFormat(language, { hour: "2-digit", minute: "2-digit", second: "2-digit" }).format(new Date(time));

function renderTraffic(app, snapshot) {
  const traffic = snapshot?.traffic;
  const total = traffic?.total || [];
  const latest = total[total.length - 1];
  const chart = lineChart({
    id: "traffic",
    series: [
      { key: "rx", label: app.t("download"), tone: "series-1", points: total.map(([time, rx]) => [time, rx]) },
      { key: "tx", label: app.t("upload"), tone: "series-2", points: total.map(([time, , tx]) => [time, tx]) },
    ],
    formatValue: formatRate,
    formatTime: clock(app.settings.language),
    hover: app.monitor.hover,
    onHover: (hover) => app.setMonitorHover(hover),
    ariaLabel: app.t("trafficChartLabel"),
  });
  return html`<section class="card span-2">
    ${cardHeader(
      app.t("liveTraffic"),
      app.t("liveTrafficHelp"),
      latest
        ? html`<span class="stat-pair"
            ><span><i class="key series-1"></i>↓ ${formatRate(latest[1])}</span
            ><span><i class="key series-2"></i>↑ ${formatRate(latest[2])}</span></span
          >`
        : nothing,
    )}
    ${!snapshot ? skeletonRows(3) : chart === nothing ? html`<p class="empty">${app.t("collectingSamples")}</p>` : chart}
    ${(traffic?.interfaces || []).length
      ? html`<table class="compact-table">
          <thead>
            <tr>
              <th>${app.t("interface")}</th>
              <th class="num">↓ ${app.t("download")}</th>
              <th class="num">↑ ${app.t("upload")}</th>
            </tr>
          </thead>
          <tbody>
            ${traffic.interfaces.map(
              (item) => html`<tr class=${item.up ? "" : "muted"}>
                <td>
                  ${item.alias}
                  ${item.owner ? chip(app.managedLabel(item.owner), "accent") : nothing}
                  ${!item.up ? chip(app.t("disconnected"), "muted") : nothing}
                </td>
                <td class="num">${formatRate(item.rxBps)}</td>
                <td class="num">${formatRate(item.txBps)}</td>
              </tr>`,
            )}
          </tbody>
        </table>
        <p class="help-text">${icon("info")} ${app.t("tunnelTrafficNote")}</p>`
      : nothing}
  </section>`;
}

function renderSignal(app, snapshot) {
  const adapters = snapshot?.signal || [];
  return html`<section class="card">
    ${cardHeader(app.t("signalHistory"), app.t("signalHistoryHelp"))}
    ${!snapshot
      ? skeletonRows(2)
      : adapters.length
        ? adapters.map((adapter) => {
            const points = (adapter.samples || []).map(([time, percent]) => [time, percent]);
            const chart = lineChart({
              id: `signal-${adapter.adapterGuid}`,
              series: [{ key: "signal", label: app.t("signal"), tone: "series-1", points }],
              max: 100,
              height: 120,
              area: false,
              formatValue: (value) => `${Math.round(value)}%`,
              formatTime: clock(app.settings.language),
              hover: app.monitor.hover,
              onHover: (hover) => app.setMonitorHover(hover),
              ariaLabel: `${app.t("signalHistory")} · ${adapter.alias}`,
            });
            return html`<div class="tool-block">
              <div class="tool-block-head">
                ${icon("wifi")}<strong>${adapter.alias}</strong>
                <span class="muted">${adapter.ssid || app.t("disconnected")}</span>
                <span class="toolbar-spacer"></span>
                <span>${adapter.signalPercent != null ? `${adapter.signalPercent}%` : "—"}</span>
              </div>
              ${chart === nothing ? html`<p class="empty">${app.t("collectingSamples")}</p>` : chart}
            </div>`;
          })
        : html`<p class="empty">${app.t("noSignalHistory")}</p>`}
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
  const snapshot = app.monitor.snapshot;
  return html`<div class="dashboard">
    ${renderTraffic(app, snapshot)} ${renderSignal(app, snapshot)} ${renderNotificationSettings(app)}
    ${renderEvents(app)}
  </div>`;
}
