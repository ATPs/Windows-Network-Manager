import { html, nothing } from "lit";
import { icon } from "../icons.js";
import { formatTime } from "../format.js";
import { buttonContent, cardHeader, emptyState } from "../ui.js";

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

export function renderProfiles(app) {
  const language = app.settings.language;
  const profiles = app.config.profiles || [];
  const recent = app.config.recentSnapshots || [];
  return html`<div class="stack">
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
    </div>
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
  </div>`;
}
