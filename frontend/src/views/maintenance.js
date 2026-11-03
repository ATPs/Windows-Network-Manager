import { html, nothing } from "lit";
import { icon } from "../icons.js";
import { formatTime } from "../format.js";
import { actionWithHelp, buttonContent, cardHeader, chip, emptyState, explainer, segmented, skeletonRows, switchInput } from "../ui.js";

const INCLUDE_KEYS = ["ip", "metric", "category", "routes", "hosts"];

function describeAdapter(app, adapter) {
  const parts = [];
  const v4 = adapter.target?.ipv4;
  if (v4) parts.push(v4.mode === "dhcp" ? app.t("ipAutomatic") : `${v4.address || "—"}/${v4.prefixLength ?? "—"}`);
  const dns = adapter.target?.dns;
  if (dns) parts.push(`DNS ${dns.mode === "manual" ? (dns.servers || []).join(", ") : app.t("dnsAutomatic")}`);
  const metric = adapter.metrics?.ipv4;
  if (metric) parts.push(`${app.t("metric")} ${metric.automaticMetric ? app.t("automaticMetric") : metric.interfaceMetric}`);
  if (adapter.networkCategory) parts.push(app.t(`category${adapter.networkCategory}`));
  if (adapter.ipError) parts.push(app.t("backupIpUnreadable"));
  return parts.join(" · ");
}

function renderBackupDetails(app, item) {
  return html`<div class="backup-details">
    <ul>
      ${item.adapters.map(
        (adapter) => html`<li>
          <strong>${adapter.alias}</strong>
          ${adapter.owner ? chip(app.managedLabel(adapter.owner), "accent") : nothing}
          <small>${describeAdapter(app, adapter)}</small>
        </li>`,
      )}
    </ul>
    <small class="field-help">
      ${app.tf("backupRoutesCount", { count: item.routes.length })} ·
      ${item.hosts?.hash ? app.tf("backupHostsEntries", { count: item.hosts.entries ?? 0 }) : app.t("backupNoHosts")}
    </small>
  </div>`;
}

function renderBackupRow(app, item) {
  const busy = app.isBusyPrefix("backup:");
  const restoreBusy = app.isBusy(`backup:restore:${item.id}`) || app.isBusy(`backup:restore:${item.id}:preview`);
  const open = app.backupDetails.has(item.id);
  return html`<div class="backup-row">
    <div class="list-row">
      ${icon(item.kind === "manual" ? "save" : "clock")}
      <span class="list-main">
        <strong>${app.backupLabel(item)}</strong>
        <small>${formatTime(item.createdAt, app.settings.language)} · ${app.tf("backupAdapterCount", { count: item.adapters.length })}${item.name && item.reason ? ` · ${app.msg(item.reason)}` : ""}</small>
      </span>
      <span class="row-buttons">
        <button class="standard compact" ?disabled=${!app.privilege.isAdmin || busy} @click=${() => app.restoreBackup(item)}>
          ${buttonContent(restoreBusy, "maintenance", app.t("restoreEllipsis"))}
        </button>
        <button class="standard compact" aria-expanded=${String(open)} @click=${() => app.toggleBackupDetails(item)}>
          ${icon(open ? "chevronUp" : "chevronDown")}${app.t("details")}
        </button>
        ${item.kind === "automatic"
          ? html`<button class="standard compact" title=${app.t("keepBackupHelp")} ?disabled=${busy} @click=${() => app.keepBackup(item)}>
              ${icon("save")}${app.t("keepBackup")}
            </button>`
          : html`<button class="standard compact" ?disabled=${busy} @click=${() => app.openNameDialog("renameBackup", item.name, item.id)}>
              ${icon("edit")}${app.t("rename")}
            </button>`}
        <button class="danger compact" ?disabled=${busy} @click=${() => app.deleteBackup(item)}>${icon("trash")}${app.t("delete")}</button>
      </span>
    </div>
    ${open ? renderBackupDetails(app, item) : nothing}
  </div>`;
}

function renderBackups(app) {
  const data = app.backups;
  const all = data?.backups || [];
  const limits = data?.limits || { manual: 50, automatic: 20 };
  const counts = {
    manual: all.filter((item) => item.kind === "manual").length,
    automatic: all.filter((item) => item.kind === "automatic").length,
  };
  const shown = all.filter((item) => item.kind === app.backupTab);
  const createBusy = app.isBusy("backup:create");
  return html`<section class="card recovery-card">
    <span class="recovery-icon">${icon("save")}</span>
    <div class="recovery-main">
      ${cardHeader(
        app.t("networkBackups"),
        app.t("networkBackupsHelp"),
        html`<button class="accent" ?disabled=${createBusy} @click=${() => app.openNameDialog("backup")}>
          ${buttonContent(createBusy, "plus", app.t("createBackup"))}
        </button>`,
      )}
      ${explainer(app, { what: app.t("backupsWhat"), when: app.t("backupsWhen"), notChanged: app.t("backupsNotIncluded") })}
      <div class="field">
        <span>${app.t("backupRestoreParts")}</span>
        <div class="check-row">
          ${INCLUDE_KEYS.map(
            (key) => html`<label class="check"
              ><input type="checkbox" .checked=${app.backupInclude[key]} @change=${(event) => app.setBackupInclude(key, event.target.checked)} /><span
                >${app.t(`backupPart_${key}`)}</span
              ></label
            >`,
          )}
        </div>
        <small class="field-help">${app.t("backupRestorePartsHelp")}</small>
      </div>
      <div class="toolbar">
        ${segmented(
          [
            ["manual", `${app.t("backupManual")} (${counts.manual}/${limits.manual})`],
            ["automatic", `${app.t("backupAutomatic")} (${counts.automatic}/${limits.automatic})`],
          ],
          app.backupTab,
          (tab) => {
            app.backupTab = tab;
            app.requestUpdate();
          },
          app.t("networkBackups"),
        )}
      </div>
      <p class="help-text">${app.t(app.backupTab === "manual" ? "backupManualHelp" : "backupAutomaticHelp")}</p>
      ${!data
        ? skeletonRows(2)
        : shown.length
          ? html`<div class="row-list">${shown.map((item) => renderBackupRow(app, item))}</div>`
          : emptyState("save", app.t(app.backupTab === "manual" ? "noManualBackups" : "noAutomaticBackups"))}
    </div>
  </section>`;
}

export function renderMaintenance(app) {
  const candidates = app.defaultsCandidates();
  const selected = app.selectedDefaults();
  const managedCount = (app.config.managedRoutes || []).length;
  const undoBusy = app.isBusy("reset") || app.isBusy("reset-preview");
  const defaultsBusy = app.isBusy("defaults") || app.isBusy("defaults-preview");
  const selectedCount = candidates.filter((item) => selected.has(item.interfaceIndex)).length;
  const setAll = (value) => {
    app.defaultsSelection = new Set(value ? candidates.map((item) => item.interfaceIndex) : []);
    app.requestUpdate();
  };
  const flushBusy = app.isBusy("fix:flushDns") || app.isBusy("fix:flushDns:preview");
  return html`<div class="stack">
    <div class="notice recovery-intro">
      ${icon("info")}
      <div>
        <strong>${app.t("recoveryIntro")}</strong>
        <ol class="recovery-steps">
          <li>${app.t("recoveryStepFlush")}</li>
          <li>${app.t("recoveryStepAdapter")}</li>
          <li>${app.t("recoveryStepUndo")}</li>
          <li>${app.t("recoveryStepBackup")}</li>
          <li>${app.t("recoveryStepDefaults")}</li>
        </ol>
      </div>
    </div>
    ${!app.privilege.isAdmin
      ? html`<p class="permission-note">${icon("shield")}${app.t("recoveryPermissionHelp")}</p>`
      : nothing}
    <section class="card recovery-card">
      <span class="recovery-icon">${icon("refresh")}</span>
      <div class="recovery-main">
        ${cardHeader(app.t("quickFixes"), app.t("quickFixesHelp"))}
        <div class="action-list">
          ${actionWithHelp(
            html`<button class="standard" ?disabled=${flushBusy} @click=${() => app.quickFix("flushDns")}>
              ${buttonContent(flushBusy, "refresh", app.t("flushDns"))}
            </button>`,
            app.t("flushDnsHelp"),
          )}
          ${actionWithHelp(
            html`<button class="standard" @click=${() => app.goToAdapters("connected")}>
              ${app.t("adapterQuickFixes")}${icon("chevronRight")}
            </button>`,
            app.t("adapterFixesHelp"),
          )}
        </div>
      </div>
    </section>
    <section class="card recovery-card">
      <span class="recovery-icon">${icon("maintenance")}</span>
      <div class="recovery-main">
        ${cardHeader(app.t("undoLastChange"), app.config.hasBaseline ? app.t("undoHelp") : app.t("noBaseline"))}
        ${explainer(app, { what: app.t("undoWhat"), when: app.t("undoWhen"), notChanged: app.t("undoNotChanged") })}
        <div>
          <button
            class="standard"
            ?disabled=${!app.config.hasBaseline || !app.privilege.isAdmin || undoBusy}
            @click=${() => app.undoLastChange()}
          >
            ${buttonContent(undoBusy, "maintenance", app.t("reviewUndo"))}
          </button>
        </div>
      </div>
    </section>
    ${renderBackups(app)}
    <section class="card recovery-card">
      <span class="recovery-icon danger">${icon("warning")}</span>
      <div class="recovery-main">
        ${cardHeader(app.t("restoreDefaults"), app.t("defaultsHelp"))}
        ${explainer(app, { what: app.t("defaultsWhat"), when: app.t("defaultsWhen"), notChanged: app.t("defaultsNotChanged") })}
        <div class="selection-header">
          <strong>${app.tf("defaultsAdapters", { selected: selectedCount, total: candidates.length })}</strong>
          <span class="button-row">
            <button class="standard compact" @click=${() => setAll(true)}>${app.t("selectAll")}</button>
            <button class="standard compact" @click=${() => setAll(false)}>${app.t("selectNone")}</button>
          </span>
        </div>
        <div class="check-grid">
          ${candidates.map(
            (item) => html`<label class="check"
              ><input
                type="checkbox"
                .checked=${selected.has(item.interfaceIndex)}
                @change=${(event) => {
                  event.target.checked
                    ? selected.add(item.interfaceIndex)
                    : selected.delete(item.interfaceIndex);
                  app.requestUpdate();
                }}
              /><span>${item.alias}</span></label
            >`,
          )}
        </div>
        ${switchInput(
          app.defaultsRemoveRoutes,
          app.tf("removeManagedRoutesCount", { count: managedCount }),
          (checked) => {
            app.defaultsRemoveRoutes = checked;
            app.requestUpdate();
          },
        )}
        <div>
          <button
            class="danger"
            ?disabled=${!app.privilege.isAdmin ||
            defaultsBusy ||
            (!selectedCount && !(app.defaultsRemoveRoutes && managedCount))}
            @click=${() => app.restoreDefaults()}
          >
            ${buttonContent(defaultsBusy, "maintenance", app.t("reviewDefaults"))}
          </button>
        </div>
      </div>
    </section>
  </div>`;
}
