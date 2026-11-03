import { html, nothing } from "lit";
import { icon } from "../icons.js";
import { formatTime } from "../format.js";
import { buttonContent, cardHeader, chip, emptyState, switchInput } from "../ui.js";

function restoreButton(app, item, recent) {
  const key = `${recent ? "recent" : "profile"}:${item.id}`;
  const busy = app.isBusy(key) || app.isBusy(`${key}:preview`);
  return html`<button
    class="standard"
    ?disabled=${!app.privilege.isAdmin || busy}
    title=${app.privilege.isAdmin ? "" : app.t("permissionHelp")}
    @click=${() => app.restoreProfile(item, recent)}
  >
    ${buttonContent(busy, "maintenance", app.t("restore"))}
  </button>`;
}

function deleteButton(app, item, recent) {
  const busy = app.isBusy(`${recent ? "delete-recent" : "delete"}:${item.id}`);
  return html`<button
    class="icon-button subtle danger-text"
    title=${app.t("delete")}
    aria-label=${`${app.t("delete")} ${item.name}`}
    ?disabled=${busy}
    @click=${() => (recent ? app.deleteRecent(item) : app.deleteProfile(item))}
  >
    ${icon("trash")}
  </button>`;
}

function renderTriggerEditor(app) {
  const draft = app.triggerDraft;
  const adapters = app.wifiAdapters();
  const adapter = adapters.find((item) => item.guid === draft.adapterGuid);
  const profiles = (adapter && app.wifiProfiles.get(adapter.alias)) || [];
  const configs = (app.config.profiles || []).filter((item) => item.id);
  const busy = app.isBusy("trigger") || app.isBusy("trigger:preview");
  return html`<div class="auto-editor">
    <div class="form-row">
      <label class="field"
        >${app.t("adapters")}
        <select @change=${(event) => app.updateTriggerDraft({ adapterGuid: event.target.value, profileName: "" })}>
          ${adapters.map((item) => html`<option value=${item.guid} ?selected=${item.guid === draft.adapterGuid}>${item.alias}</option>`)}
        </select>
      </label>
      <label class="field"
        >${app.t("whenConnectedTo")}
        <select @change=${(event) => app.updateTriggerDraft({ profileName: event.target.value })}>
          <option value="">${app.t("wifiAutoChooseProfile")}</option>
          ${profiles.map((name) => html`<option value=${name} ?selected=${name === draft.profileName}>${name}</option>`)}
        </select>
      </label>
      <label class="field"
        >${app.t("applySavedConfig")}
        <select @change=${(event) => app.updateTriggerDraft({ savedConfigId: event.target.value })}>
          ${configs.map((item) => html`<option value=${item.id} ?selected=${item.id === draft.savedConfigId}>${item.name}</option>`)}
        </select>
      </label>
    </div>
    <div class="button-row">
      <button class="accent" ?disabled=${busy || !draft.profileName || !draft.savedConfigId} @click=${() => app.saveTrigger()}>
        ${buttonContent(busy, null, app.t("save"))}
      </button>
      <button class="standard" @click=${() => { app.triggerDraft = null; app.requestUpdate(); }}>${app.t("cancel")}</button>
    </div>
  </div>`;
}

function renderTriggers(app) {
  const triggers = app.config.networkTriggers || [];
  const configName = (id) => (app.config.profiles || []).find((item) => item.id === id)?.name || id;
  const adapterName = (guid) =>
    app.wifiAdapters().find((item) => String(item.guid).toLowerCase().includes(String(guid).toLowerCase()))?.alias || app.t("wifiAutoConfiguredAdapter");
  return html`<section class="card">
    ${cardHeader(
      app.t("automaticConfigs"),
      app.t("automaticConfigsHelp"),
      app.triggerDraft
        ? html``
        : html`<button class="standard" ?disabled=${!(app.config.profiles || []).length || !app.wifiAdapters().length} @click=${() => app.openTriggerEditor()}>
            ${icon("plus")}${app.t("addTrigger")}
          </button>`,
    )}
    ${!app.privilege.isAdmin ? html`<p class="permission-note">${icon("shield")}${app.t("triggerAdminHelp")}</p>` : nothing}
    ${app.triggerDraft ? renderTriggerEditor(app) : nothing}
    ${triggers.length
      ? html`<div class="row-list">
          ${triggers.map(
            (trigger) => html`<div class="list-row">
              ${icon("wifi")}
              <span class="list-main">
                <strong>${trigger.profileName} → ${configName(trigger.savedConfigId)}</strong>
                <small>${adapterName(trigger.adapterGuid)}</small>
              </span>
              ${trigger.enabled ? chip(app.t("enabled"), "accent") : chip(app.t("disabled"), "muted")}
              <span class="row-buttons">
                ${switchInput(trigger.enabled, "", (enabled) => app.toggleTrigger(trigger, enabled), app.isBusyPrefix("trigger"))}
                <button
                  class="icon-button subtle danger-text"
                  title=${app.t("remove")}
                  aria-label=${`${app.t("remove")} ${trigger.profileName}`}
                  @click=${() => app.removeTrigger(trigger)}
                >
                  ${icon("trash")}
                </button>
              </span>
            </div>`,
          )}
        </div>`
      : app.triggerDraft
        ? nothing
        : html`<p class="empty">${app.t("noTriggers")}</p>`}
  </section>`;
}

export function renderProfiles(app) {
  const language = app.settings.language;
  const profiles = app.config.profiles || [];
  const recent = app.config.recentSnapshots || [];
  return html`<div class="stack">
    <p class="notice">
      ${icon("info")}<span>${app.t("profilesVsBackups")}</span>
      <button class="link-button" @click=${() => app.setView("maintenance")}>${app.t("openNetworkBackups")}</button>
    </p>
    <div class="toolbar">
      <button
        class="accent"
        ?disabled=${app.isBusy("snapshot")}
        @click=${() => app.openNameDialog("snapshot")}
      >
        ${buttonContent(app.isBusy("snapshot"), "save", app.t("saveSnapshot"))}
      </button>
      <button
        class="standard"
        ?disabled=${!app.network.overview || !app.network.routes || app.isBusy("profile")}
        @click=${() => app.openNameDialog("profile")}
      >
        ${buttonContent(app.isBusy("profile"), "profiles", app.t("saveProfile"))}
      </button>
      <span class="toolbar-spacer"></span>
      <button class="subtle" ?disabled=${app.isBusy("import") || app.isBusy("import:preview")} @click=${() => app.importConfig()}>
        ${buttonContent(app.isBusy("import:preview"), "save", app.t("importConfig"))}
      </button>
      <button class="subtle" ?disabled=${app.isBusy("export")} @click=${() => app.exportConfig()}>
        ${buttonContent(app.isBusy("export"), "copy", app.t("exportConfig"))}
      </button>
    </div>
    <p class="help-text">${app.t("importExportHelp")}</p>
    ${!app.privilege.isAdmin
      ? html`<p class="permission-note">${icon("shield")}${app.t("restorePermissionHelp")}</p>`
      : nothing}
    <section class="card">
      ${cardHeader(app.t("savedProfiles"), app.t("savedProfilesHelp"))}
      ${profiles.length
        ? html`<div class="row-list">
            ${profiles.map(
              (item) => html`<div class="list-row">
                ${icon("profiles")}
                <span class="list-main">
                  <strong>${item.name}</strong>
                  <small>${app.t("updated")} ${formatTime(item.updatedAt || item.createdAt, language)}</small>
                </span>
                <span class="row-buttons">${restoreButton(app, item, false)}${deleteButton(app, item, false)}</span>
              </div>`,
            )}
          </div>`
        : emptyState("profiles", app.t("noProfiles"), app.t("noProfilesHelp"))}
    </section>
    <section class="card">
      ${cardHeader(app.t("recentSnapshots"), app.t("recentSnapshotsHelp"))}
      ${recent.length
        ? html`<div class="row-list">
            ${recent.map(
              (item) => html`<div class="list-row">
                ${icon("clock")}
                <span class="list-main">
                  <strong>${item.name}</strong>
                  <small
                    >${app.t("lastUsed")} ${formatTime(item.lastUsedAt, language)}${item.createdAt
                      ? ` · ${app.t("created")} ${formatTime(item.createdAt, language)}`
                      : ""}${item.useCount
                      ? ` · ${app.tf("useCount", { count: item.useCount })}`
                      : ""}</small
                  >
                </span>
                <span class="row-buttons">${restoreButton(app, item, true)}${deleteButton(app, item, true)}</span>
              </div>`,
            )}
          </div>`
        : emptyState("clock", app.t("noSnapshots"), app.t("noSnapshotsHelp"))}
    </section>
    ${renderTriggers(app)}
  </div>`;
}
