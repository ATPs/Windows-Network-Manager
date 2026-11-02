import { html, nothing } from "lit";
import { icon } from "../icons.js";
import { buttonContent, cardHeader, emptyState, skeletonRows, switchInput } from "../ui.js";

const HOST_NAME = /^(?=.{1,253}$)([A-Za-z0-9_](?:[A-Za-z0-9_-]{0,61}[A-Za-z0-9_])?)(\.[A-Za-z0-9_](?:[A-Za-z0-9_-]{0,61}[A-Za-z0-9_])?)*$/;

/** Host names separated by spaces or commas, validated like the backend. */
export function parseHostNames(text) {
  const names = String(text || "").split(/[\s,]+/).filter(Boolean);
  return { names, valid: names.length > 0 && names.length <= 20 && names.every((name) => HOST_NAME.test(name)) };
}

export const isHostsAddress = (value) => {
  const text = String(value || "").trim();
  const v4 = /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/;
  return v4.test(text) || (/^[0-9a-f:]+$/i.test(text) && text.includes(":"));
};

function renderEntry(app, line) {
  const names = parseHostNames(line.names.join(" "));
  return html`<tr class=${line.enabled ? "" : "muted"}>
    <td>${switchInput(Boolean(line.enabled), "", (enabled) => app.updateHostsLine(line.key, { enabled }))}</td>
    <td>
      <input
        class="mono ${isHostsAddress(line.ip) ? "" : "invalid"}"
        aria-label=${app.t("hostsAddress")}
        spellcheck="false"
        .value=${line.ip}
        @change=${(event) => app.updateHostsLine(line.key, { ip: event.target.value.trim() })}
      />
    </td>
    <td>
      <input
        class="mono ${names.valid ? "" : "invalid"}"
        aria-label=${app.t("hostsNames")}
        spellcheck="false"
        .value=${line.names.join(" ")}
        @change=${(event) => app.updateHostsLine(line.key, { names: parseHostNames(event.target.value).names })}
      />
    </td>
    <td>
      <input
        aria-label=${app.t("hostsComment")}
        maxlength="200"
        .value=${line.comment || ""}
        @change=${(event) => app.updateHostsLine(line.key, { comment: event.target.value.trim() })}
      />
    </td>
    <td class="row-actions">
      <button
        class="icon-button subtle danger-text"
        title=${app.t("remove")}
        aria-label=${`${app.t("remove")} ${line.names.join(" ")}`}
        @click=${() => app.removeHostsLine(line.key)}
      >
        ${icon("trash")}
      </button>
    </td>
  </tr>`;
}

export function renderHosts(app) {
  const hosts = app.hosts;
  const lines = app.hostsEdit || [];
  const entries = lines.filter((line) => line.kind === "entry");
  const rawCount = lines.length - entries.length;
  const dirty = app.hostsDirty();
  const invalid = entries.some((line) => !isHostsAddress(line.ip) || !parseHostNames(line.names.join(" ")).valid);
  const busy = app.isBusy("hosts") || app.isBusy("hosts:preview");
  let form;
  return html`<div class="stack">
    <p class="notice warning">${icon("warning")}${app.t("hostsWarning")}</p>
    <div class="toolbar">
      <button class="standard" ?disabled=${app.isBusy("hosts:load")} @click=${() => app.loadHosts()}>
        ${buttonContent(app.isBusy("hosts:load"), "refresh", dirty ? app.t("discardChanges") : app.t("reload"))}
      </button>
      <span class="toolbar-spacer"></span>
      <span class="muted">${hosts?.path || ""}</span>
      <button
        class="accent"
        ?disabled=${!dirty || invalid || busy || !app.privilege.isAdmin}
        title=${app.privilege.isAdmin ? "" : app.t("permissionHelp")}
        @click=${() => app.saveHosts()}
      >
        ${buttonContent(busy, "save", app.t("reviewChanges"))}
      </button>
    </div>
    ${!app.privilege.isAdmin ? html`<p class="permission-note">${icon("shield")}${app.t("hostsPermissionHelp")}</p>` : nothing}
    <section class="card">
      ${cardHeader(app.t("hostsEntries"), app.tf("hostsEntriesHelp", { count: rawCount }))}
      ${!hosts
        ? skeletonRows(4)
        : entries.length
          ? html`<div class="table-scroll tall">
              <table class="compact-table hosts-table">
                <thead>
                  <tr>
                    <th>${app.t("enabled")}</th>
                    <th>${app.t("hostsAddress")}</th>
                    <th>${app.t("hostsNames")}</th>
                    <th>${app.t("hostsComment")}</th>
                    <th><span class="visually-hidden">${app.t("actions")}</span></th>
                  </tr>
                </thead>
                <tbody>
                  ${entries.map((line) => renderEntry(app, line))}
                </tbody>
              </table>
            </div>`
          : emptyState("hosts", app.t("hostsEmpty"))}
      <form
        class="tool-form hosts-add"
        @submit=${(event) => {
          event.preventDefault();
          form = event.currentTarget;
          const data = new FormData(form);
          const ip = String(data.get("ip") || "").trim();
          const parsed = parseHostNames(data.get("names"));
          if (!isHostsAddress(ip) || !parsed.valid) return app.notify(app.t("hostsInvalidEntry"), "error");
          app.addHostsLine({ ip, names: parsed.names, comment: String(data.get("comment") || "").trim() });
          form.reset();
        }}
      >
        <input name="ip" class="mono" spellcheck="false" aria-label=${app.t("hostsAddress")} placeholder=${app.t("hostsAddress")} />
        <input name="names" class="mono" spellcheck="false" aria-label=${app.t("hostsNames")} placeholder=${app.t("hostsNamesPlaceholder")} />
        <input name="comment" maxlength="200" aria-label=${app.t("hostsComment")} placeholder=${app.t("hostsComment")} />
        <button class="standard" type="submit" ?disabled=${!hosts}>${icon("plus")}${app.t("addEntry")}</button>
      </form>
    </section>
  </div>`;
}
