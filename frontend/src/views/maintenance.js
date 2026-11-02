import { html, nothing } from "lit";
import { icon } from "../icons.js";
import { buttonContent, cardHeader, switchInput } from "../ui.js";

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
  return html`<div class="stack">
    ${!app.privilege.isAdmin
      ? html`<p class="permission-note">${icon("shield")}${app.t("recoveryPermissionHelp")}</p>`
      : nothing}
    <section class="card recovery-card">
      <span class="recovery-icon">${icon("maintenance")}</span>
      <div class="recovery-main">
        ${cardHeader(app.t("undoLastChange"), app.config.hasBaseline ? app.t("undoHelp") : app.t("noBaseline"))}
        <button
          class="standard"
          ?disabled=${!app.config.hasBaseline || !app.privilege.isAdmin || undoBusy}
          @click=${() => app.undoLastChange()}
        >
          ${buttonContent(undoBusy, "maintenance", app.t("reviewUndo"))}
        </button>
      </div>
    </section>
    <section class="card recovery-card">
      <span class="recovery-icon danger">${icon("warning")}</span>
      <div class="recovery-main">
        ${cardHeader(app.t("restoreDefaults"), app.t("defaultsHelp"))}
        <div class="selection-header">
          <strong>${app.tf("defaultsAdapters", { selected: selectedCount, total: candidates.length })}</strong>
          <span class="button-row">
            <button class="subtle compact" @click=${() => setAll(true)}>${app.t("selectAll")}</button>
            <button class="subtle compact" @click=${() => setAll(false)}>${app.t("selectNone")}</button>
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
