import { html, nothing } from "lit";
import { keyed } from "lit/directives/keyed.js";
import { icon } from "../icons.js";
import { isIpv4, isIpv4Cidr, isIpv6, isIpv6Cidr, routeIdentity } from "../format.js";
import { buttonContent, chip, emptyState, segmented, skeletonRows, switchInput } from "../ui.js";

function sortHeader(app, key, label) {
  const active = app.routeSort.key === key;
  const direction = active ? (app.routeSort.dir === 1 ? "ascending" : "descending") : "none";
  return html`<th aria-sort=${direction}>
    <button
      class="sort-button ${active ? "active" : ""}"
      @click=${() => {
        app.routeSort = { key, dir: active ? -app.routeSort.dir : 1 };
        app.saveRoutePrefs();
        app.requestUpdate();
      }}
    >
      ${label}${active ? icon(app.routeSort.dir === 1 ? "chevronUp" : "chevronDown") : nothing}
    </button>
  </th>`;
}

function renderRow(app, route, managed, showFamily) {
  const busy = app.routeBusy();
  const disabled = !app.privilege.isAdmin || busy;
  const isManaged = managed.has(routeIdentity(route));
  // Routes of EasyTier, v2rayN and similar software are left to that program.
  const owned = (route.managedBy || route.captureRoute) && !isManaged;
  const effective = route.effectiveMetric ?? route.routeMetric;
  return html`<tr class=${route.defaultRoute ? "default-route" : ""}>
    ${showFamily ? html`<td class="muted">${route.addressFamily}</td>` : nothing}
    <td>
      <span class="mono">${route.destinationPrefix}</span>
      ${route.defaultRoute ? chip(app.t("defaultRoute"), "accent") : nothing}
    </td>
    <td class="mono">${route.onLink ? html`<span class="muted">${app.t("onLink")}</span>` : route.nextHop}</td>
    <td>${route.interfaceAlias} <span class="muted">#${route.interfaceIndex}</span></td>
    <td
      class="num"
      title=${app.tf("metricBreakdownFull", {
        route: route.routeMetric ?? "—",
        iface: route.interfaceMetric ?? "—",
        total: effective ?? "—",
      })}
    >
      ${effective ?? "—"}
    </td>
    <td class="chips">
      ${chip(route.persistent ? app.t("persistent") : app.t("temporary"), route.persistent ? "" : "muted")}
      ${route.systemManaged ? chip(app.t("system"), "muted") : nothing}
      ${isManaged ? chip(app.t("appManaged"), "accent") : nothing}
      ${route.managedBy ? chip(app.ownerLabel(route.managedBy), "muted") : nothing}
    </td>
    <td class="row-actions">
      ${route.removable && !owned
        ? html`<button
              class="icon-button subtle"
              title=${app.t("edit")}
              aria-label=${`${app.t("edit")} ${route.destinationPrefix}`}
              ?disabled=${disabled}
              @click=${() => app.startRouteEdit(route)}
            >
              ${icon("edit")}
            </button>
            ${!route.persistent
              ? html`<button
                  class="icon-button subtle"
                  title=${app.t("makePersistent")}
                  aria-label=${`${app.t("makePersistent")} ${route.destinationPrefix}`}
                  ?disabled=${disabled}
                  @click=${() => app.makePersistent(route)}
                >
                  ${icon("pin")}
                </button>`
              : nothing}
            <button
              class="icon-button subtle danger-text"
              title=${app.t("remove")}
              aria-label=${`${app.t("remove")} ${route.destinationPrefix}`}
              ?disabled=${disabled}
              @click=${() => app.removeRoute(route)}
            >
              ${icon("trash")}
            </button>`
        : html`<span class="muted" title=${owned ? app.t("routeProtected") : app.t("routeReadOnly")}>—</span>`}
    </td>
  </tr>`;
}

function renderDomainRoutes(app) {
  const entries = app.config.domainRoutes || [];
  const draft = app.domainDraft;
  const interfaces = app.interfaces().filter((item) => item.metrics?.ipv4);
  const index = draft.interfaceIndex || String(interfaces[0]?.interfaceIndex ?? "");
  const busy = app.isBusyPrefix("domain:");
  const ready = draft.domain.trim() && isIpv4(draft.nextHop) && index;
  return html`<section class="card">
    ${cardHeader(app.t("domainRoutes"), app.t("domainRoutesHelp"))}
    ${entries.length
      ? html`<div class="row-list">
          ${entries.map(
            (entry) => html`<div class="list-row">
              ${icon("globe")}
              <span class="list-main">
                <strong>${entry.domain}</strong>
                <small>${entry.nextHop} · #${entry.interfaceIndex} · ${app.tf("domainAddresses", { count: (entry.addresses || []).length })}</small>
              </span>
              <span class="row-buttons">
                <button class="subtle compact" ?disabled=${busy || !app.privilege.isAdmin} @click=${() => app.domainRoute(entry, "save")}>
                  ${icon("refresh")}${app.t("refresh")}
                </button>
                <button
                  class="icon-button subtle danger-text"
                  title=${app.t("remove")}
                  aria-label=${`${app.t("remove")} ${entry.domain}`}
                  ?disabled=${busy || !app.privilege.isAdmin}
                  @click=${() => app.domainRoute(entry, "remove")}
                >
                  ${icon("trash")}
                </button>
              </span>
            </div>`,
          )}
        </div>`
      : nothing}
    <form
      class="tool-form domain-form"
      @submit=${(event) => {
        event.preventDefault();
        app.domainRoute({ domain: draft.domain.trim(), nextHop: draft.nextHop.trim(), interfaceIndex: Number(index), routeMetric: Number(draft.routeMetric) || 5 });
      }}
    >
      <input
        class="mono"
        spellcheck="false"
        placeholder="example.com"
        aria-label=${app.t("domain")}
        .value=${draft.domain}
        @input=${(event) => app.updateDomainDraft({ domain: event.target.value })}
      />
      <input
        class="mono"
        spellcheck="false"
        placeholder=${app.t("nextHop")}
        aria-label=${app.t("nextHop")}
        .value=${draft.nextHop}
        @input=${(event) => app.updateDomainDraft({ nextHop: event.target.value })}
      />
      <select aria-label=${app.t("interface")} @change=${(event) => app.updateDomainDraft({ interfaceIndex: event.target.value })}>
        ${interfaces.map((item) => html`<option value=${item.interfaceIndex} ?selected=${String(item.interfaceIndex) === index}>${item.alias} (#${item.interfaceIndex})</option>`)}
      </select>
      <button class="standard" type="submit" ?disabled=${!ready || busy || !app.privilege.isAdmin}>
        ${buttonContent(app.isBusyPrefix(`domain:${draft.domain.trim()}`), "plus", app.t("addDomainRoute"))}
      </button>
    </form>
  </section>`;
}

export function renderRoutes(app) {
  const all = app.network.routes?.routes || [];
  const rows = app.routeRows();
  const managed = app.managedRouteKeys();
  const showFamily = app.routeFamily === "all";
  return html`<div class="stack">
    <div class="toolbar">
      <label class="search-box"
        >${icon("search")}<input
          type="search"
          data-page-search
          placeholder=${app.t("routeSearch")}
          aria-label=${app.t("routeSearch")}
          .value=${app.routeSearch}
          @input=${(event) => {
            app.routeSearch = event.target.value;
            app.requestUpdate();
          }}
      /></label>
      ${segmented(
        [
          ["IPv4", "IPv4"],
          ["IPv6", "IPv6"],
          ["all", app.t("filterAll")],
        ],
        app.routeFamily,
        (value) => {
          app.routeFamily = value;
          app.saveRoutePrefs();
          app.requestUpdate();
        },
        app.t("addressFamily"),
      )}
      ${switchInput(app.settings.showAllRoutes, app.t("showAllRoutes"), (checked) =>
        app.updateSettings({ showAllRoutes: checked }),
      )}
      ${app.routeStaticOnly
        ? html`<button
            class="filter-chip selected"
            @click=${() => {
              app.routeStaticOnly = false;
              app.requestUpdate();
            }}
          >
            ${app.t("staticRoutes")}${icon("close")}
          </button>`
        : nothing}
      <span class="toolbar-spacer"></span>
      <button
        class="accent"
        ?disabled=${!app.privilege.isAdmin}
        title=${app.privilege.isAdmin ? "" : app.t("permissionHelp")}
        @click=${() => app.openRouteDrawer()}
      >
        ${icon("plus")}${app.t("addRoute")}
      </button>
    </div>
    ${!app.privilege.isAdmin
      ? html`<p class="permission-note">${icon("shield")}${app.t("routePermissionHelp")}</p>`
      : nothing}
    <div class="table-card">
      ${!app.network.routes
        ? skeletonRows(6)
        : html`<table>
            <thead>
              <tr>
                ${showFamily ? sortHeader(app, "family", app.t("family")) : nothing}
                ${sortHeader(app, "destination", app.t("destination"))}
                ${sortHeader(app, "nextHop", app.t("nextHop"))}
                ${sortHeader(app, "interface", app.t("interface"))}
                ${sortHeader(app, "metric", app.t("metric"))} ${sortHeader(app, "type", app.t("type"))}
                <th><span class="visually-hidden">${app.t("actions")}</span></th>
              </tr>
            </thead>
            <tbody>
              ${rows.length
                ? rows.map((route) => renderRow(app, route, managed, showFamily))
                : html`<tr>
                    <td colspan=${showFamily ? 7 : 6}>
                      ${emptyState("routes", app.t("noRoutes"), all.length ? app.t("noRoutesHelp") : "")}
                    </td>
                  </tr>`}
            </tbody>
          </table>`}
    </div>
    <p class="help-text">${app.t("routeMetricHelp")}</p>
    ${renderDomainRoutes(app)}
  </div>`;
}

function validateRoute(app, draft) {
  if (!draft) return {};
  const errors = {};
  const ipv6 = draft.addressFamily === "IPv6";
  if (draft.destinationPrefix && !(ipv6 ? isIpv6Cidr : isIpv4Cidr)(draft.destinationPrefix))
    errors.destinationPrefix = app.t("invalidCidr");
  if (draft.nextHop && !(ipv6 ? isIpv6 : isIpv4)(draft.nextHop))
    errors.nextHop = app.t(ipv6 ? "invalidIpv6" : "invalidIpv4");
  const metric = Number(draft.routeMetric);
  if (draft.routeMetric !== "" && draft.routeMetric != null && (!Number.isInteger(metric) || metric < 1 || metric > 9999))
    errors.routeMetric = app.t("invalidMetric");
  return errors;
}

export function renderRouteDrawer(app) {
  const route = app.editingRoute;
  const family = route?.addressFamily || app.routeDraftFamily || "IPv4";
  const ipv6 = family === "IPv6";
  const metricKey = ipv6 ? "ipv6" : "ipv4";
  const routeInterfaces = app.interfaces().filter((item) => item.metrics?.[metricKey]);
  const draft = app.routeDraft;
  const errors = validateRoute(app, draft);
  const invalid = Object.keys(errors).length > 0;
  const busy = app.routeBusy() || app.isBusyPrefix("route:add:preview") || app.isBusyPrefix("route:edit:preview");
  const selectedIndex = Number(draft?.interfaceIndex ?? route?.interfaceIndex ?? routeInterfaces[0]?.interfaceIndex);
  const selectedInterface = app.interfaces().find((item) => item.interfaceIndex === selectedIndex);
  const interfaceMetric = selectedInterface?.metrics?.[metricKey]?.interfaceMetric;
  const routeMetric = Number(draft?.routeMetric ?? route?.routeMetric ?? 5);
  const field = (name, label, input, help = "") => html`<label class="field ${errors[name] ? "invalid" : ""}"
    >${label}${input}${errors[name]
      ? html`<small class="field-error">${errors[name]}</small>`
      : help
        ? html`<small class="field-help">${help}</small>`
        : nothing}</label
  >`;
  return html`<header class="drawer-header">
      <h2 id="route-drawer-heading">${app.t(route ? "editRoute" : "addRoute")}</h2>
      <button class="icon-button subtle" aria-label=${app.t("close")} @click=${() => app.closeRouteDrawer()}>
        ${icon("close")}
      </button>
    </header>
    ${keyed(
      app.routeFormVersion,
      html`<form
        class="drawer-body route-form"
        novalidate
        @submit=${(event) => app.addRoute(event)}
        @input=${(event) => {
          app.routeDraft = app.routeFormValues(event.currentTarget);
          app.requestUpdate();
        }}
      >
        <p class="help-text">${app.t(route ? "editRouteHelp" : "addRouteHelp")}</p>
        <input type="hidden" name="addressFamily" .value=${family} />
        ${route
          ? nothing
          : segmented(
              [
                ["IPv4", "IPv4"],
                ["IPv6", "IPv6"],
              ],
              family,
              (value) => app.setRouteDraftFamily(value),
              app.t("addressFamily"),
            )}
        ${field(
          "destinationPrefix",
          app.t("destination"),
          html`<input
            name="destinationPrefix"
            class="mono"
            .value=${route?.destinationPrefix || ""}
            placeholder=${ipv6 ? "2001:db8::/32" : "10.0.0.0/8"}
            autocomplete="off"
            spellcheck="false"
            required
            data-autofocus
          />`,
        )}
        ${field(
          "nextHop",
          app.t("nextHop"),
          html`<input
            name="nextHop"
            class="mono"
            .value=${route?.nextHop || ""}
            placeholder=${ipv6 ? "fe80::1" : "192.168.1.1"}
            autocomplete="off"
            spellcheck="false"
            required
          />`,
        )}
        ${field(
          "interfaceIndex",
          app.t("interface"),
          html`<select name="interfaceIndex">
            ${route && !routeInterfaces.some((item) => item.interfaceIndex === route.interfaceIndex)
              ? html`<option value=${route.interfaceIndex} selected>
                  ${route.interfaceAlias || app.t("interface")} (#${route.interfaceIndex})
                </option>`
              : nothing}
            ${routeInterfaces.map(
              (item) =>
                html`<option value=${item.interfaceIndex} ?selected=${route?.interfaceIndex === item.interfaceIndex}>
                  ${item.alias} (#${item.interfaceIndex})
                </option>`,
            )}
          </select>`,
        )}
        ${field(
          "routeMetric",
          app.t("routeMetric"),
          html`<input
            name="routeMetric"
            type="number"
            min="1"
            max="9999"
            .value=${String(route?.routeMetric ?? 5)}
            required
          />`,
          interfaceMetric != null && Number.isFinite(routeMetric)
            ? app.tf("effectiveMetricHint", {
                route: routeMetric,
                iface: interfaceMetric,
                total: routeMetric + Number(interfaceMetric),
              })
            : app.t("routeMetricHelp"),
        )}
        <label class="check"
          ><input name="persistent" type="checkbox" .checked=${Boolean(route?.persistent)} />
          <span>${app.t("persistent")}<small class="field-help">${app.t("persistentHelp")}</small></span></label
        >
        ${!app.privilege.isAdmin
          ? html`<p class="permission-note">${icon("shield")}${app.t("permissionHelp")}</p>`
          : nothing}
        <div class="drawer-actions">
          <button type="button" class="standard" ?disabled=${app.routeBusy()} @click=${() => app.closeRouteDrawer()}>
            ${app.t("cancel")}
          </button>
          <button type="submit" class="accent" ?disabled=${!app.privilege.isAdmin || busy || invalid}>
            ${buttonContent(busy, null, app.t(route ? "saveRoute" : "addRoute"))}
          </button>
        </div>
      </form>`,
    )}`;
}
