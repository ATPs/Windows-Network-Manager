import { html, nothing } from "lit";
import { icon } from "../icons.js";
import { buttonContent, cardHeader, chip, emptyState, segmented, skeletonRows, switchInput } from "../ui.js";

const FORBIDDEN = /[*?"<>|\u0000-\u001f]/;

/**
 * Normalizes a program entry like the backend: `chrome.exe`, a full path to an
 * .exe, or a folder ending with a slash. Returns "" when invalid.
 */
export function normalizeProgramEntry(value) {
  const entry = String(value || "").trim();
  if (!entry || entry.length > 260 || FORBIDDEN.test(entry)) return "";
  if (!/[\\/:]/.test(entry)) return /^.+\.exe$/i.test(entry) ? entry : "";
  let path = entry.replace(/\\/g, "/");
  if (!/^[A-Za-z]:\//.test(path)) return "";
  path = path[0].toUpperCase() + path.slice(1);
  if (path.slice(2).includes(":") || path.includes("//") || path.split("/").some((part) => part === ".." || part === ".")) return "";
  if (path.endsWith("/")) return path.length > 3 ? path : "";
  return /\.exe$/i.test(path) ? path : "";
}

const validHost = (host) =>
  /^localhost$/i.test(host) || /^(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)(\.(25[0-5]|2[0-4]\d|1\d\d|[1-9]?\d)){3}$/.test(host);

/** The port check only applies to a proxy running on this PC. */
const isLoopback = (host) => /^localhost$/i.test(host.trim()) || /^127\./.test(host.trim());

/** Field errors for a rule draft (keys: name, programs, port, host). */
export function appRuleErrors(draft) {
  if (!draft) return {};
  const errors = {};
  if (!draft.name.trim() || draft.name.trim().length > 64) errors.name = true;
  if (!draft.programs.length || draft.programs.length > 32) errors.programs = true;
  if (draft.target === "local") {
    const port = Number(draft.localProxy.port);
    if (!Number.isInteger(port) || port < 1 || port > 65535) errors.port = true;
    if (!validHost(draft.localProxy.host.trim())) errors.host = true;
  }
  return errors;
}

function programKind(app, program) {
  if (program.endsWith("/")) return app.t("programFolder");
  return program.includes("/") ? app.t("programPath") : app.t("programName");
}

function targetLabel(app, rule) {
  if (rule.target === "local" && rule.localProxy)
    return `${rule.localProxy.protocol.toUpperCase()} ${rule.localProxy.host}:${rule.localProxy.port}`;
  return app.t(`appTarget_${rule.target}`);
}

function renderRule(app, rule) {
  const busy = app.isBusyPrefix("apps:");
  return html`<div class="list-row app-rule ${rule.enabled ? "" : "muted"}">
    ${switchInput(rule.enabled, "", (enabled) => app.saveAppRule({ ...rule, enabled }))}
    <span class="list-main">
      <strong>${rule.name}</strong>
      <small class="mono">${rule.programs.join("  ·  ")}</small>
    </span>
    ${chip(targetLabel(app, rule), rule.target === "block" ? "" : "accent")}
    <span class="row-buttons">
      <button class="icon-button subtle" title=${app.t("edit")} aria-label=${`${app.t("edit")} ${rule.name}`} ?disabled=${busy} @click=${() => app.editAppRule(rule)}>
        ${icon("edit")}
      </button>
      <button
        class="icon-button subtle danger-text"
        title=${app.t("remove")}
        aria-label=${`${app.t("remove")} ${rule.name}`}
        ?disabled=${busy}
        @click=${() => app.removeAppRule(rule)}
      >
        ${icon("trash")}
      </button>
    </span>
  </div>`;
}

function renderPortCheck(app) {
  const check = app.portCheck;
  if (!check) return nothing;
  const kind = check.loop || !check.listening ? "warning" : "info";
  return html`<p class="notice ${kind === "warning" ? "warning" : ""}">${icon(kind)}${app.msg(check.messageCode || { text: check.message })}</p>`;
}

function renderEditor(app) {
  const draft = app.appRuleDraft;
  const errors = appRuleErrors(draft);
  const entryValid = Boolean(normalizeProgramEntry(draft.entry));
  const saving = app.isBusy("apps:save");
  const running = app.runningPrograms;
  return html`<section class="card app-rule-editor">
    ${cardHeader(app.t(draft.isNew ? "newAppRule" : "editAppRule"), app.t("appRuleEditorHelp"))}
    <label class="field ${errors.name && draft.name ? "invalid" : ""}">
      ${app.t("ruleName")}
      <input
        .value=${draft.name}
        maxlength="64"
        placeholder=${app.t("ruleNamePlaceholder")}
        @input=${(event) => app.updateAppRuleDraft({ name: event.target.value })}
      />
    </label>
    <div class="field">
      <span>${app.t("programs")}</span>
      ${draft.programs.length
        ? html`<div class="row-list">
            ${draft.programs.map(
              (program) => html`<div class="list-row">
                ${icon(program.endsWith("/") ? "profiles" : "apps")}
                <span class="list-main">
                  <span class="mono">${program}</span>
                  <small>${programKind(app, program)}</small>
                </span>
                <span class="row-buttons">
                  ${program.includes("/") && !program.endsWith("/")
                    ? html`<button class="subtle compact" title=${app.t("useFolderHelp")} @click=${() => app.useProgramFolder(program)}>
                        ${app.t("useFolder")}
                      </button>`
                    : nothing}
                  <button
                    class="icon-button subtle danger-text"
                    title=${app.t("remove")}
                    aria-label=${`${app.t("remove")} ${program}`}
                    @click=${() => app.removeAppRuleProgram(program)}
                  >
                    ${icon("close")}
                  </button>
                </span>
              </div>`,
            )}
          </div>`
        : html`<p class="muted">${app.t("noProgramsYet")}</p>`}
      <form
        class="tool-form"
        @submit=${(event) => {
          event.preventDefault();
          app.addAppRuleProgram(draft.entry);
        }}
      >
        <input
          class="mono ${draft.entry && !entryValid ? "invalid" : ""}"
          spellcheck="false"
          placeholder=${app.t("programEntryPlaceholder")}
          aria-label=${app.t("programs")}
          .value=${draft.entry}
          @input=${(event) => app.updateAppRuleDraft({ entry: event.target.value })}
        />
        <button class="standard" type="submit" ?disabled=${!entryValid}>${icon("plus")}${app.t("add")}</button>
        <button class="standard" type="button" ?disabled=${app.isBusy("apps:pick")} @click=${() => app.browseProgram()}>
          ${buttonContent(app.isBusy("apps:pick"), "search", app.t("browseProgram"))}
        </button>
      </form>
      <div class="tool-form">
        ${running
          ? html`<select
              aria-label=${app.t("runningPrograms")}
              @change=${(event) => {
                if (event.target.value) app.addAppRuleProgram(event.target.value);
                event.target.value = "";
              }}
            >
              <option value="">${app.tf("pickRunningProgram", { count: running.length })}</option>
              ${running.map((item) => html`<option value=${item.path}>${item.name} — ${item.path}</option>`)}
            </select>`
          : nothing}
        <button class="subtle compact" type="button" ?disabled=${app.isBusy("apps:running")} @click=${() => app.loadRunningPrograms()}>
          ${buttonContent(app.isBusy("apps:running"), "refresh", app.t(running ? "refreshRunningPrograms" : "showRunningPrograms"))}
        </button>
      </div>
      <small class="field-help">${app.t("programsHelp")}</small>
    </div>
    <div class="field">
      <span>${app.t("appTarget")}</span>
      ${segmented(
        ["proxy", "direct", "block", "local"].map((target) => [target, app.t(`appTarget_${target}`)]),
        draft.target,
        (target) => app.updateAppRuleDraft({ target }),
        app.t("appTarget"),
      )}
      <small class="field-help">${app.t(`appTargetHelp_${draft.target}`)}</small>
    </div>
    ${draft.target === "local"
      ? html`<div class="tool-form local-proxy-form">
            <select
              aria-label=${app.t("proxyProtocol")}
              @change=${(event) => app.updateAppRuleDraft({ localProxy: { ...draft.localProxy, protocol: event.target.value } })}
            >
              ${["socks", "http"].map((protocol) => html`<option value=${protocol} ?selected=${draft.localProxy.protocol === protocol}>${protocol.toUpperCase()}</option>`)}
            </select>
            <input
              class="mono ${errors.host ? "invalid" : ""}"
              spellcheck="false"
              aria-label=${app.t("proxyHost")}
              placeholder=${app.t("proxyHost")}
              .value=${draft.localProxy.host}
              @input=${(event) => app.updateAppRuleDraft({ localProxy: { ...draft.localProxy, host: event.target.value } })}
            />
            <input
              class="mono ${errors.port && draft.localProxy.port ? "invalid" : ""}"
              type="number"
              min="1"
              max="65535"
              aria-label=${app.t("proxyPort")}
              placeholder=${app.t("proxyPort")}
              .value=${draft.localProxy.port}
              @input=${(event) => app.updateAppRuleDraft({ localProxy: { ...draft.localProxy, port: event.target.value } })}
            />
            <button
              class="standard"
              type="button"
              title=${isLoopback(draft.localProxy.host) ? "" : app.t("checkPortLocalOnly")}
              ?disabled=${errors.port || !isLoopback(draft.localProxy.host) || app.isBusy("apps:port")}
              @click=${() => app.checkLocalProxyPort()}
            >
              ${buttonContent(app.isBusy("apps:port"), "search", app.t("checkPort"))}
            </button>
          </div>
          ${renderPortCheck(app)}`
      : nothing}
    <div class="drawer-actions">
      <button class="standard" type="button" ?disabled=${saving} @click=${() => app.closeAppRuleDraft()}>${app.t("cancel")}</button>
      <button class="accent" type="button" ?disabled=${saving || Object.keys(errors).length > 0} @click=${() => app.saveAppRule()}>
        ${buttonContent(saving, "save", app.t("saveRule"))}
      </button>
    </div>
  </section>`;
}

function renderServerStep(app, server) {
  if (server.link)
    return html`<li>
      ${app.tf("v2raynStepSocks", { remark: server.remark })}
      <div class="copy-row"><code class="mono">${server.link}</code>
        <button class="subtle compact" @click=${() => app.copyText(server.link)}>${icon("copy")}${app.t("copy")}</button>
      </div>
    </li>`;
  return html`<li>${app.tf("v2raynStepHttp", { remark: server.remark, host: server.host, port: server.port })}</li>`;
}

function renderExport(app) {
  const exported = app.appRules?.export;
  if (!exported) return nothing;
  return html`<section class="card">
    ${cardHeader(app.t("v2raynSetup"), app.t("v2raynSetupHelp"))}
    ${switchInput(app.appRulesOthersDirect, app.t("othersDirect"), (enabled) => app.setAppRulesOthersDirect(enabled))}
    <p class="help-text">${app.t("othersDirectHelp")}</p>
    ${exported.ruleCount
      ? html`<ol class="steps">
            ${(exported.servers || []).map((server) => renderServerStep(app, server))}
            <li>
              ${app.t("v2raynStepImport")}
              <div class="copy-row">
                <button class="accent" @click=${() => app.copyText(exported.rulesJson)}>${icon("copy")}${app.tf("copyRules", { count: exported.ruleCount })}</button>
              </div>
            </li>
            <li>${app.t("v2raynStepTop")}</li>
            <li>${app.t("v2raynStepCheck")}</li>
          </ol>
          ${exported.usesFolders ? html`<p class="notice warning">${icon("warning")}${app.t("foldersNeedXray")}</p>` : nothing}
          <details>
            <summary>${app.t("showRulesJson")}</summary>
            <pre class="mono code-block">${exported.rulesJson}</pre>
          </details>`
      : html`<p class="muted">${app.t("noEnabledAppRules")}</p>`}
  </section>`;
}

export function renderApps(app) {
  const rules = app.appRules?.rules;
  return html`<div class="stack">
    <p class="notice">${icon("info")}${app.t("appRulesIntro")}</p>
    <section class="card">
      ${cardHeader(
        app.t("appRules"),
        app.t("appRulesHelp"),
        html`<button class="accent" ?disabled=${Boolean(app.appRuleDraft)} @click=${() => app.newAppRule()}>${icon("plus")}${app.t("newAppRule")}</button>`,
      )}
      ${!rules
        ? skeletonRows(3)
        : rules.length
          ? html`<div class="row-list">${rules.map((rule) => renderRule(app, rule))}</div>`
          : emptyState("apps", app.t("noAppRules"), app.t("noAppRulesHelp"))}
    </section>
    ${app.appRuleDraft ? renderEditor(app) : nothing}
    ${renderExport(app)}
  </div>`;
}
