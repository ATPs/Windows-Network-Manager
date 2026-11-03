import { html } from "lit";
import { icon } from "../icons.js";
import { buttonContent, cardHeader, segmented, switchInput } from "../ui.js";
import { formatBytes } from "../format.js";

/** Text size steps; Rust maps each to a webview zoom factor. */
export const FONT_SIZES = ["small", "default", "large", "xlarge"];

const shortcuts = [
  ["Ctrl+1 … Ctrl+9", "shortcutViews"],
  ["F5", "shortcutRefresh"],
  ["Ctrl+= / Ctrl+- / Ctrl+0", "shortcutFontSize"],
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

const RETENTION_DAYS = [1, 7, 14, 30, 90, 180, 365];

/** Saved monitor history: on/off, retention, latency and the storage folder. */
export function renderMonitorHistorySettings(app) {
  const options = app.settings.monitor || {};
  const storage = app.monitor?.storage;
  const update = (change) => app.updateMonitorSettings(change);
  const sourceLabel = { install: "storageInstall", appData: "storageAppData", custom: "storageCustom" };
  return html`<section class="card">
    ${cardHeader(app.t("monitorHistorySettings"), app.t("monitorHistorySettingsHelp"))}
    <div class="setting-row">
      <div><strong>${app.t("historyEnabled")}</strong><small>${app.t("historyEnabledHelp")}</small></div>
      ${switchInput(options.historyEnabled !== false, "", (checked) => update({ historyEnabled: checked }))}
    </div>
    <div class="setting-row">
      <div><strong>${app.t("retention")}</strong><small>${app.t("retentionHelp")}</small></div>
      <select
        aria-label=${app.t("retention")}
        @change=${(event) => update({ retentionDays: Number(event.target.value) })}
      >
        ${RETENTION_DAYS.map(
          (days) => html`<option value=${days} ?selected=${(options.retentionDays ?? 30) === days}>${app.tf("retentionDays", { days })}</option>`,
        )}
      </select>
    </div>
    <div class="setting-row">
      <div><strong>${app.t("latencyEnabled")}</strong><small>${app.t("latencyEnabledHelp")}</small></div>
      ${switchInput(options.latencyEnabled !== false, "", (checked) => update({ latencyEnabled: checked }))}
    </div>
    <div class="setting-row">
      <div><strong>${app.t("latencyInterval")}</strong><small>${app.t("latencyIntervalHelp")}</small></div>
      ${segmented(
        [10, 30, 60].map((seconds) => [seconds, app.tf("everySeconds", { seconds })]),
        options.latencyIntervalSeconds ?? 10,
        (seconds) => update({ latencyIntervalSeconds: seconds }),
        app.t("latencyInterval"),
      )}
    </div>
    <div class="setting-row">
      <div><strong>${app.t("latencyPublicDns")}</strong><small>${app.t("latencyPublicDnsHelp")}</small></div>
      ${switchInput(Boolean(options.latencyPublicDns), "", (checked) => update({ latencyPublicDns: checked }), options.latencyEnabled === false)}
    </div>
    <div class="setting-row storage-row">
      <div>
        <strong>${app.t("historyFolder")}</strong>
        <small class="mono">${storage?.path || "—"}</small>
        ${storage
          ? html`<small>${app.t(sourceLabel[storage.source] || "storageAppData")} · ${app.tf("storageUsage", { size: formatBytes(storage.sizeBytes), days: storage.days })}</small>`
          : ""}
        ${storage?.fallbackReason
          ? html`<small class="inline-error">${icon("warning")}${app.t(`storageFallback_${storage.fallbackReason}`)}</small>`
          : ""}
      </div>
      <span class="button-row">
        <button class="standard compact" ?disabled=${app.isBusy("monitor:folder")} @click=${() => app.chooseMonitorFolder()}>
          ${buttonContent(app.isBusy("monitor:folder"), "profiles", app.t("changeFolder"))}
        </button>
        ${options.folder
          ? html`<button class="standard compact" @click=${() => app.resetMonitorFolder()}>${app.t("useDefaultFolder")}</button>`
          : ""}
        <button class="standard compact" @click=${() => app.openMonitorFolder()}>${app.t("openFolder")}</button>
        <button class="danger compact" ?disabled=${!storage?.days} @click=${() => app.clearMonitorHistory()}>${app.t("clearMonitorHistory")}</button>
      </span>
    </div>
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
        <div><strong>${app.t("fontSize")}</strong><small>${app.t("fontSizeHelp")}</small></div>
        ${segmented(
          FONT_SIZES.map((size) => [size, app.t(`fontSize_${size}`)]),
          app.settings.fontSize || "default",
          (fontSize) => app.updateSettings({ fontSize }),
          app.t("fontSize"),
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
    ${renderMonitorHistorySettings(app)} ${renderNotificationSettings(app)}
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
