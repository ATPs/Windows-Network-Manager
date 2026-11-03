import { html, nothing } from "lit";
import { icon } from "./icons.js";
import { signalLevel } from "./format.js";

export const spinner = () => html`<span class="spinner" aria-hidden="true"></span>`;

export const pill = (kind, text, iconName = null) =>
  html`<span class="status-pill ${kind}"
    >${iconName ? icon(iconName) : nothing}${text}</span
  >`;

export const chip = (text, kind = "") =>
  html`<span class="chip ${kind}">${text}</span>`;

export function signalBars(percent, label) {
  const level = signalLevel(percent);
  return html`<span
    class="signal-bars level-${level}"
    role="img"
    aria-label=${label}
    title=${label}
    ><i></i><i></i><i></i><i></i
  ></span>`;
}

/** Button content that swaps its icon for a spinner while busy. */
export const buttonContent = (busy, iconName, text) =>
  html`${busy ? spinner() : iconName ? icon(iconName) : nothing}<span
      >${text}</span
    >`;

export function copyButton(app, value) {
  if (!value || value === "—") return nothing;
  return html`<button
    type="button"
    class="icon-button subtle copy-button"
    title=${app.t("copy")}
    aria-label=${`${app.t("copy")} ${value}`}
    @click=${(event) => {
      event.stopPropagation();
      app.copyText(value);
    }}
  >
    ${icon("copy")}
  </button>`;
}

export function fact(label, value, { copy = null, app = null, wide = false } = {}) {
  const text = value == null || value === "" ? "—" : value;
  return html`<div class="fact ${wide ? "wide" : ""}">
    <small>${label}</small>
    <span class="fact-value"
      ><span>${text}</span>${copy && app ? copyButton(app, copy) : nothing}</span
    >
  </div>`;
}

export const emptyState = (iconName, title, detail = "", action = nothing) =>
  html`<div class="empty-state">
    <span class="empty-icon">${icon(iconName)}</span>
    <strong>${title}</strong>
    ${detail ? html`<p>${detail}</p>` : nothing} ${action}
  </div>`;

export const cardHeader = (title, detail = "", actions = nothing) =>
  html`<div class="card-header">
    <div>
      <h2>${title}</h2>
      ${detail ? html`<p>${detail}</p>` : nothing}
    </div>
    ${actions === nothing ? nothing : html`<div class="card-actions">${actions}</div>`}
  </div>`;

export const skeletonRows = (count = 3) =>
  html`<div class="skeleton-list" aria-hidden="true">
    ${Array.from({ length: count }, () => html`<div class="skeleton"></div>`)}
  </div>`;

export const switchInput = (checked, label, onChange, disabled = false) =>
  html`<label class="switch"
    ><input
      type="checkbox"
      role="switch"
      .checked=${checked}
      ?disabled=${disabled}
      @change=${(event) => onChange(event.target.checked)}
    /><span class="switch-track" aria-hidden="true"></span><span>${label}</span></label
  >`;

export function segmented(options, value, onChange, label) {
  return html`<div class="segmented" role="radiogroup" aria-label=${label}>
    ${options.map(
      ([optionValue, text]) =>
        html`<button
          type="button"
          role="radio"
          aria-checked=${String(optionValue === value)}
          class=${optionValue === value ? "selected" : ""}
          @click=${() => onChange(optionValue)}
        >
          ${text}
        </button>`,
    )}
  </div>`;
}

/** A button with a visible one-line explanation under it. */
export const actionWithHelp = (button, help) =>
  html`<div class="action-with-help">${button}<small class="field-help">${help}</small></div>`;

/** "What it does / When to use it / What it does not change" for a recovery action. */
export const explainer = (app, { what, when, notChanged }) =>
  html`<dl class="explain">
    <dt>${app.t("explainWhat")}</dt>
    <dd>${what}</dd>
    ${when ? html`<dt>${app.t("explainWhen")}</dt><dd>${when}</dd>` : nothing}
    ${notChanged ? html`<dt>${app.t("explainNotChanged")}</dt><dd>${notChanged}</dd>` : nothing}
  </dl>`;
