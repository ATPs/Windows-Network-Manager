import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  previewQuickFix: vi.fn(),
  applyQuickFix: vi.fn(),
  startupStatus: vi.fn(),
  setStartWithWindows: vi.fn(),
  exportConfig: vi.fn(),
  importConfigPreview: vi.fn(),
  importConfigApply: vi.fn(),
  profiles: vi.fn(),
  wifiAutoStatus: vi.fn(),
}));

vi.mock("./api.js", () => ({ api }));
vi.mock("lit", () => ({
  LitElement: class {
    requestUpdate() {}
    get updateComplete() { return Promise.resolve(); }
  },
  html: () => undefined,
  svg: () => undefined,
  nothing: undefined,
}));
vi.mock("lit/directives/keyed.js", () => ({ keyed: (_key, value) => value }));

let NetworkManagerApp;
beforeAll(async () => {
  vi.stubGlobal("customElements", { define() {} });
  vi.stubGlobal("localStorage", { getItem: () => null, setItem() {} });
  vi.stubGlobal("document", { hidden: false, documentElement: { dataset: {} } });
  ({ NetworkManagerApp } = await import("./network-manager-app.js"));
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(() => {
  vi.resetAllMocks();
  api.profiles.mockResolvedValue({});
  api.wifiAutoStatus.mockResolvedValue({});
});

function app() {
  const instance = new NetworkManagerApp();
  instance.querySelector = () => null;
  instance.refresh = vi.fn().mockResolvedValue(undefined);
  return instance;
}

const wlan = { interfaceIndex: 5, alias: "WLAN" };

describe("Quick fixes", () => {
  it("previews, asks for confirmation and applies the confirmed fix", async () => {
    const instance = app();
    const fix = { action: "renewDhcp", interfaceIndex: 5 };
    api.previewQuickFix.mockResolvedValue({ fix, confirmationToken: "t", notices: [{ code: "fix.renewDhcp.interrupt", text: "WLAN drops" }] });
    api.applyQuickFix.mockResolvedValue({ message: "DHCP lease renewed on WLAN." });
    await instance.quickFix("renewDhcp", wlan);
    expect(api.previewQuickFix).toHaveBeenCalledWith({ action: "renewDhcp", interfaceIndex: 5 });
    expect(api.applyQuickFix).not.toHaveBeenCalled();
    expect(instance.confirmation.title).toBe("Renew the DHCP lease?");
    expect(instance.confirmation.destructive).toBe(true);
    await instance.confirm();
    expect(api.applyQuickFix).toHaveBeenCalledWith(fix, "t");
    expect(instance.activity[0].label).toBe("Renew DHCP · WLAN");
    expect(instance.refresh).toHaveBeenCalled();
  });

  it("flushes DNS without refreshing adapters", async () => {
    const instance = app();
    api.previewQuickFix.mockResolvedValue({ fix: { action: "flushDns", interfaceIndex: null }, confirmationToken: "t" });
    api.applyQuickFix.mockResolvedValue({});
    await instance.quickFix("flushDns");
    expect(instance.confirmation.destructive).toBe(false);
    await instance.confirm();
    expect(instance.refresh).not.toHaveBeenCalled();
  });

  it("does not ask for confirmation when the backend refuses a protected adapter", async () => {
    const instance = app();
    api.previewQuickFix.mockRejectedValue({ code: "PROTECTED_ADAPTER", message: "et_6_0qmn is managed by EasyTier." });
    await instance.quickFix("restartAdapter", { interfaceIndex: 33, alias: "et_6_0qmn" });
    expect(instance.confirmation).toBeNull();
    expect(instance.toasts[0].kind).toBe("error");
  });
});

describe("Import and export", () => {
  it("ignores a cancelled file dialog", async () => {
    const instance = app();
    api.importConfigPreview.mockResolvedValue({ cancelled: true });
    await instance.importConfig();
    expect(instance.confirmation).toBeNull();
    api.exportConfig.mockResolvedValue({ cancelled: true });
    await instance.exportConfig();
    expect(instance.toasts).toHaveLength(0);
  });

  it("shows what an import changes and applies the previewed data", async () => {
    const instance = app();
    const data = { settings: { language: "zh-CN", theme: "dark" }, profiles: [], wifiAutoConnect: [] };
    api.importConfigPreview.mockResolvedValue({
      import: data,
      summary: { settings: true, profilesAdded: 2, profilesReplaced: 0, rules: 1 },
      confirmationToken: "t",
      fileName: "backup.json",
    });
    api.importConfigApply.mockResolvedValue({ settings: data.settings, message: "Configuration imported." });
    await instance.importConfig();
    expect(instance.confirmation.rows.map((row) => row.after)).toEqual(["Replaced", "2 added", "1 rules (disabled)"]);
    await instance.confirm();
    expect(api.importConfigApply).toHaveBeenCalledWith(data, "t");
    expect(instance.settings.language).toBe("zh-CN");
    expect(instance.settings.notifications.enabled).toBe(false);
  });

  it("stores the startup status returned after a change", async () => {
    const instance = app();
    api.setStartWithWindows.mockResolvedValue({ enabled: true, matchesCurrent: true, available: true });
    await instance.setStartWithWindows(true);
    expect(api.setStartWithWindows).toHaveBeenCalledWith(true);
    expect(instance.startup.enabled).toBe(true);
  });
});
