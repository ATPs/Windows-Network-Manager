import { html } from "lit";
import { icon } from "../icons.js";
import { cardHeader, segmented, switchInput } from "../ui.js";

const shortcuts = [
  ["Ctrl+1 … Ctrl+9", "shortcutViews"],
  ["F5", "shortcutRefresh"],
  ["/", "shortcutSearch"],
  ["Esc", "shortcutClose"],
];

/** Opt-in tray notifications for network events (shared with Monitor). */
export function renderNotificationSettings(app) {
  const options = app.settings.notifications || {};
  const row = (key, label, help) => html`<div class="setting-row">
    <div><strong>${app.t(label)}</strong><small>${app.t(help)}</small></div>
    ${switchInput(Boolean(options[key]), "", (checked) => app.updateNotifications({ [key]: checked }), !options.enabled)}
  </div>`;
  return html`<section class="card">
    ${cardHeader(app.t("notifications"), app.t("notificationsHelp"))}
    <div class="setting-row">
      <div><strong>${app.t("notificationsEnabled")}</strong><small>${app.t("notificationsEnabledHelp")}</small></div>
      ${switchInput(Boolean(options.enabled), "", (checked) => app.updateNotifications({ enabled: checked }))}
    </div>
    ${row("adapters", "notifyAdapters", "notifyAdaptersHelp")}
    ${row("wifi", "notifyWifi", "notifyWifiHelp")}
    ${row("defaultRoute", "notifyDefaultRoute", "notifyDefaultRouteHelp")}
    ${row("tunnels", "notifyTunnels", "notifyTunnelsHelp")}
  </section>`;
}

export function renderSettings(app) {
  return html`<div class="stack settings">
    <section class="card">
      ${cardHeader(app.t("appearance"))}
      <div class="setting-row">
        <div><strong>${app.t("theme")}</strong><small>${app.t("themeHelp")}</small></div>
        ${segmented(
          [
            ["system", app.t("system")],
            ["light", app.t("light")],
            ["dark", app.t("dark")],
          ],
          app.settings.theme,
          (theme) => app.updateSettings({ theme }),
          app.t("theme"),
        )}
      </div>
      <div class="setting-row">
        <div><strong>${app.t("language")}</strong><small>${app.t("languageHelp")}</small></div>
        <select
          aria-label=${app.t("language")}
          .value=${app.settings.language}
          @change=${(event) => app.updateSettings({ language: event.target.value })}
        >
          <option value="en" ?selected=${app.settings.language === "en"}>English</option>
          <option value="zh-CN" ?selected=${app.settings.language === "zh-CN"}>中文（简体）</option>
        </select>
      </div>
    </section>
    <section class="card">
      ${cardHeader(app.t("display"))}
      <div class="setting-row">
        <div><strong>${app.t("showSystem")}</strong><small>${app.t("showSystemHelp")}</small></div>
        ${switchInput(app.settings.showSystemInterfaces, "", (checked) =>
          app.updateSettings({ showSystemInterfaces: checked }),
        )}
      </div>
      <div class="setting-row">
        <div><strong>${app.t("showAllRoutes")}</strong><small>${app.t("showAllRoutesHelp")}</small></div>
        ${switchInput(app.settings.showAllRoutes, "", (checked) =>
          app.updateSettings({ showAllRoutes: checked }),
        )}
      </div>
    </section>
    <section class="card">
      ${cardHeader(app.t("startup"))}
      <div class="setting-row">
        <div>
          <strong>${app.t("startWithWindows")}</strong>
          <small>${app.startup && !app.startup.available ? app.t("startupDevHelp") : app.t("startWithWindowsHelp")}</small>
          ${app.startup?.enabled && !app.startup.matchesCurrent && app.startup.available
            ? html`<small class="inline-error">${icon("warning")}${app.t("startupMismatch")}</small>`
            : ""}
        </div>
        ${switchInput(
          Boolean(app.startup?.enabled),
          "",
          (checked) => app.setStartWithWindows(checked),
          !app.startup?.available || app.isBusy("startup"),
        )}
      </div>
    </section>
    ${renderNotificationSettings(app)}
    <section class="card">
      ${cardHeader(app.t("keyboardShortcuts"))}
      <dl class="shortcut-list">
        ${shortcuts.map(
          ([keys, label]) => html`<dt><kbd>${keys}</kbd></dt><dd>${app.t(label)}</dd>`,
        )}
      </dl>
    </section>
    <section class="card about">
      <span class="brand-mark large">${icon("globe")}</span>
      <div>
        <h2>${app.t("appName")}</h2>
        <p class="muted">${app.version ? `${app.t("version")} ${app.version}` : ""}</p>
        <p class="help-text">${app.t("aboutPrivacy")}</p>
        <p class="help-text">${app.t("aboutTray")}</p>
      </div>
    </section>
  </div>`;
}
