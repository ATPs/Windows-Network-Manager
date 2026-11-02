import { html } from "lit";
import { icon } from "../icons.js";
import { formatTime } from "../format.js";
import { buttonContent, cardHeader, emptyState, fact } from "../ui.js";

function interfaceFindings(app, item) {
  const reasons = [];
  if (item.deviations?.manualIpv4Metric) reasons.push(app.t("findingManualIpv4"));
  if (item.deviations?.manualIpv6Metric) reasons.push(app.t("findingManualIpv6"));
  if (item.deviations?.adapterDisabled) reasons.push(app.t("findingDisabled"));
  return reasons.length ? reasons : [app.t("configurationChange")];
}

export function diagnosticFindings(app, report) {
  const changes = report?.changes || {};
  return [
    ...(changes.interfaces || []).map((item) => ({
      kind: "warning",
      title: item.alias || `#${item.interfaceIndex}`,
      detail: interfaceFindings(app, item).join(" · "),
      go: () => {
        const live = app.interfaces().find((candidate) => candidate.interfaceIndex === item.interfaceIndex);
        live ? app.goToAdapter(live) : app.goToAdapters();
      },
    })),
    ...(changes.staticRoutes || []).map((route) => ({
      kind: "info",
      title: route.destinationPrefix,
      detail: `${app.t("staticRouteFound")} · ${route.nextHop} · ${route.interfaceAlias || `#${route.interfaceIndex}`}`,
      go: () => app.goToRoutes({ search: route.destinationPrefix }),
    })),
  ];
}

const countBy = (items, key) =>
  items.reduce((counts, item) => {
    const value = key(item) || "Other";
    counts[value] = (counts[value] || 0) + 1;
    return counts;
  }, {});

export function diagnosticsText(app, report) {
  const os = report.os || {};
  const lines = [
    `${app.t("appName")} ${app.version ? `v${app.version}` : ""}`.trim(),
    `${app.t("generatedAt")}: ${formatTime(report.generatedAt, app.settings.language)}`,
    `${app.t("operatingSystem")}: ${[os.Caption, os.Version, os.BuildNumber && `build ${os.BuildNumber}`, os.OSArchitecture].filter(Boolean).join(" · ")}`,
    `${app.t("privilege")}: ${report.privilege?.isAdmin ? app.t("administrator") : app.t("standardUser")}`,
    "",
    `## ${app.t("diagnosticSummary")}`,
  ];
  const findings = diagnosticFindings(app, report);
  lines.push(...(findings.length ? findings.map((item) => `- ${item.title}: ${item.detail}`) : [app.t("noChanges")]));
  lines.push("", `## ${app.t("adapters")}`);
  for (const item of report.overview?.interfaces || []) {
    const ips = [...(item.ipv4Addresses || []), ...(item.ipv6Addresses || [])]
      .map((ip) => `${ip.ipAddress}/${ip.prefixLength}`)
      .join(", ");
    lines.push(
      `- ${item.alias} (#${item.interfaceIndex}, ${item.type}, ${item.adminStatus}) ${ips || "—"}; ${app.t("gateway")}: ${[...(item.ipv4Gateways || []), ...(item.ipv6Gateways || [])].join(", ") || "—"}; IPv4 ${app.t("metric")}: ${item.metrics?.ipv4 ? (item.metrics.ipv4.automaticMetric ? `auto (${item.metrics.ipv4.interfaceMetric})` : item.metrics.ipv4.interfaceMetric) : "—"}`,
    );
  }
  lines.push("", `## ${app.t("defaultPriority")}`);
  for (const route of report.routes?.defaultCandidates || []) {
    lines.push(`- ${route.addressFamily} ${route.destinationPrefix} → ${route.nextHop} via ${route.interfaceAlias} (${app.t("metric")} ${route.effectiveMetric ?? route.routeMetric})`);
  }
  return lines.join("\n");
}

export function renderDiagnostics(app) {
  const report = app.diagnostics;
  const busy = app.isBusy("diagnostics");
  const generate = html`<button class="accent" ?disabled=${busy} @click=${() => app.generateDiagnostics()}>
    ${buttonContent(busy, "diagnostics", report ? app.t("regenerate") : app.t("generateDiagnostics"))}
  </button>`;
  if (!report)
    return html`<section class="card">
      ${emptyState("diagnostics", app.t("diagnosticsIdleTitle"), app.t("diagnosticsIdle"), generate)}
    </section>`;
  const os = report.os || {};
  const findings = diagnosticFindings(app, report);
  const interfaces = report.overview?.interfaces || [];
  const routes = report.routes?.routes || [];
  const byType = countBy(interfaces, (item) => item.type);
  const byFamily = countBy(routes, (route) => route.addressFamily);
  return html`<div class="stack">
    <div class="toolbar">
      ${generate}
      <button class="standard" @click=${() => app.copyText(diagnosticsText(app, report))}>
        ${icon("copy")}${app.t("copyText")}
      </button>
      <button class="standard" @click=${() => app.copyText(JSON.stringify(report, null, 2))}>
        ${icon("copy")}${app.t("copyJson")}
      </button>
      <span class="toolbar-spacer"></span>
      <span class="muted">${app.t("generatedAt")} ${formatTime(report.generatedAt, app.settings.language)}</span>
    </div>
    <div class="dashboard">
      <section class="card">
        ${cardHeader(app.t("system"))}
        <div class="fact-grid two">
          ${fact(app.t("operatingSystem"), os.Caption, { wide: true })}
          ${fact(app.t("version"), [os.Version, os.BuildNumber && `build ${os.BuildNumber}`].filter(Boolean).join(" · "))}
          ${fact(app.t("architecture"), os.OSArchitecture)}
          ${fact(app.t("privilege"), report.privilege?.isAdmin ? app.t("administrator") : app.t("standardUser"))}
          ${fact(app.t("appVersion"), app.version ? `v${app.version}` : null)}
          ${fact(app.t("schemaVersion"), report.app?.schemaVersion)}
          ${fact(app.t("recentSnapshots"), report.app?.recentSnapshots)}
        </div>
      </section>
      <section class="card">
        ${cardHeader(app.t("inventory"))}
        <div class="fact-grid two">
          ${Object.entries(byType).map(([type, count]) => fact(type, count))}
          ${Object.entries(byFamily).map(([family, count]) => fact(`${family} ${app.t("routes")}`, count))}
          ${fact(app.t("staticRoutes"), (report.changes?.staticRoutes || []).length)}
        </div>
      </section>
      <section class="card span-2">
        ${cardHeader(app.t("diagnosticSummary"), findings.length ? app.t("findingsHelp") : "")}
        ${findings.length
          ? html`<ul class="finding-list">
              ${findings.map(
                (finding) => html`<li class=${finding.kind}>
                  ${icon(finding.kind)}
                  <span class="list-main"><strong>${finding.title}</strong><small>${finding.detail}</small></span>
                  <button class="subtle compact" @click=${finding.go}>${app.t("goTo")}${icon("chevronRight")}</button>
                </li>`,
              )}
            </ul>`
          : html`<div class="all-clear">${icon("success")}<span>${app.t("noChanges")}</span></div>`}
      </section>
      <section class="card span-2">
        <details>
          <summary>${app.t("technicalDetails")}</summary>
          <pre class="code-block">${JSON.stringify(report, null, 2)}</pre>
        </details>
      </section>
    </div>
  </div>`;
}

