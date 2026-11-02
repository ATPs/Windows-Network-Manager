import { html, nothing } from "lit";
import { icon } from "../icons.js";
import { isIpv4 } from "../format.js";
import { buttonContent, segmented, skeletonRows } from "../ui.js";

/** Dotted subnet mask for a prefix length, or "" when out of range. */
export function prefixToMask(prefix) {
  const bits = Number(prefix);
  if (!Number.isInteger(bits) || bits < 0 || bits > 32) return "";
  const mask = bits === 0 ? 0 : (0xffffffff << (32 - bits)) >>> 0;
  return [24, 16, 8, 0].map((shift) => (mask >>> shift) & 255).join(".");
}

const isIp = (value) => isIpv4(value) || /^[0-9a-f:]+$/i.test(value) && value.includes(":");

/** Client-side checks; the backend repeats them with stricter rules. */
export function validateIpDraft(app, draft) {
  const errors = {};
  if (draft.ipv4Mode === "static") {
    if (!isIpv4(draft.address)) errors.address = app.t("invalidIpv4");
    const prefix = Number(draft.prefixLength);
    if (!Number.isInteger(prefix) || prefix < 1 || prefix > 32) errors.prefixLength = app.t("invalidPrefix");
    if (draft.gateway.trim() && !isIpv4(draft.gateway)) errors.gateway = app.t("invalidIpv4");
  }
  if (draft.dnsMode === "manual") {
    const servers = draft.dnsServers.split(/[\s,;]+/).filter(Boolean);
    if (!servers.length || servers.length > 4 || !servers.every(isIp)) errors.dnsServers = app.t("invalidDns");
  }
  return errors;
}

export function renderIpDrawer(app) {
  const editor = app.ipEditor;
  const item = editor.item;
  const draft = editor.draft;
  const errors = draft ? validateIpDraft(app, draft) : {};
  const busy = app.isBusy(`ip:preview:${item.interfaceIndex}`) || app.isBusy(`ip:${item.interfaceIndex}`);
  const field = (name, label, input, help = "") => html`<label class="field ${errors[name] ? "invalid" : ""}"
    >${label}${input}${errors[name]
      ? html`<small class="field-error">${errors[name]}</small>`
      : help
        ? html`<small class="field-help">${help}</small>`
        : nothing}</label
  >`;
  const text = (name, value, extra = {}) => html`<input
    class="mono"
    autocomplete="off"
    spellcheck="false"
    .value=${value}
    ?disabled=${busy}
    inputmode=${extra.inputmode || "text"}
    @input=${(event) => app.updateIpDraft({ [name]: event.target.value })}
  />`;
  return html`<header class="drawer-header">
      <h2 id="ip-drawer-heading">${app.t("editIpSettings")} · ${item.alias}</h2>
      <button class="icon-button subtle" aria-label=${app.t("close")} @click=${() => app.closeIpEditor()}>
        ${icon("close")}
      </button>
    </header>
    <form
      class="drawer-body route-form"
      novalidate
      @submit=${(event) => {
        event.preventDefault();
        app.saveIpSettings();
      }}
    >
      <p class="notice warning">${icon("warning")}${app.t("ipEditorWarning")}</p>
      ${editor.loading || !draft
        ? skeletonRows(4)
        : html`<h3 class="form-section">IPv4</h3>
            ${segmented(
              [
                ["dhcp", app.t("ipAutomatic")],
                ["static", app.t("ipManual")],
              ],
              draft.ipv4Mode,
              (ipv4Mode) => app.updateIpDraft({ ipv4Mode }),
              "IPv4",
            )}
            ${draft.ipv4Mode === "static"
              ? html`${field("address", app.t("ipv4Address"), text("address", draft.address))}
                ${field(
                  "prefixLength",
                  app.t("prefixLength"),
                  html`<input
                    type="number"
                    min="1"
                    max="32"
                    .value=${draft.prefixLength}
                    ?disabled=${busy}
                    @input=${(event) => app.updateIpDraft({ prefixLength: event.target.value })}
                  />`,
                  prefixToMask(draft.prefixLength) ? app.tf("subnetMaskHint", { mask: prefixToMask(draft.prefixLength) }) : "",
                )}
                ${field("gateway", app.t("gateway"), text("gateway", draft.gateway), app.t("gatewayHelp"))}`
              : html`<p class="help-text">${app.t("ipAutomaticHelp")}</p>`}
            <h3 class="form-section">DNS</h3>
            ${segmented(
              [
                ["automatic", app.t("dnsAutomatic")],
                ["manual", app.t("dnsManual")],
              ],
              draft.dnsMode,
              (dnsMode) => {
                if (dnsMode === "automatic" && draft.ipv4Mode === "static") return app.notify(app.t("staticNeedsDns"), "warning");
                app.updateIpDraft({ dnsMode });
              },
              "DNS",
            )}
            ${draft.dnsMode === "manual"
              ? field("dnsServers", app.t("dnsServersField"), text("dnsServers", draft.dnsServers), app.t("dnsServersHelp"))
              : html`<p class="help-text">${app.t("dnsAutomaticHelp")}</p>`}`}
      ${!app.privilege.isAdmin ? html`<p class="permission-note">${icon("shield")}${app.t("permissionHelp")}</p>` : nothing}
      <div class="drawer-actions">
        <button type="button" class="standard" ?disabled=${busy} @click=${() => app.closeIpEditor()}>${app.t("cancel")}</button>
        <button
          type="submit"
          class="accent"
          ?disabled=${!app.privilege.isAdmin || busy || !draft || Object.keys(errors).length > 0}
        >
          ${buttonContent(busy, null, app.t("reviewChanges"))}
        </button>
      </div>
    </form>`;
}
