import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  previewWifiAuto: vi.fn(),
  saveWifiAuto: vi.fn(),
  disableWifiAuto: vi.fn(),
  removeWifiAuto: vi.fn(),
  wifiAutoStatus: vi.fn(),
}));

vi.mock("./api.js", () => ({ api }));
vi.mock("lit", () => ({
  LitElement: class {
    requestUpdate() {}
    get updateComplete() { return Promise.resolve(); }
  },
  html: () => undefined,
  nothing: undefined,
}));
vi.mock("lit/directives/keyed.js", () => ({ keyed: (_key, value) => value }));

let NetworkManagerApp;
beforeAll(async () => {
  vi.stubGlobal("customElements", { define() {} });
  vi.stubGlobal("localStorage", { getItem: () => null, setItem() {} });
  ({ NetworkManagerApp } = await import("./network-manager-app.js"));
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(() => vi.resetAllMocks());

function app() {
  const instance = new NetworkManagerApp();
  instance.refresh = vi.fn().mockResolvedValue(undefined);
  instance.refreshConfig = vi.fn().mockResolvedValue(undefined);
  return instance;
}

const adapter = {
  guid: "ABC-123",
  alias: "Wi-Fi",
  wifi: { currentProfile: "Office" },
};

describe("Wi-Fi auto-connect rules", () => {
  it("does not silently choose the first saved network for a disconnected adapter", () => {
    const instance = app();
    const draft = instance.wifiAutoDraft("ABC-123", { alias: "Wi-Fi", wifi: {} }, { profiles: ["Office"] });
    expect(draft.profileName).toBe("");
  });

  it("resolves adapter identity from Wi-Fi data or matching discovery info", () => {
    const instance = app();
    instance.wifiAuto = {
      adapters: [{ adapterGuid: "discovered-guid", interfaceName: "Wi-Fi" }],
      rules: [],
      statuses: [],
    };

    expect(instance.wifiAutoGuid({ alias: "Wi-Fi", wifi: { guid: "wifi-guid" } })).toBe("wifi-guid");
    expect(instance.wifiAutoGuid({ alias: "Wi-Fi", wifi: {} })).toBe("discovered-guid");
  });

  it("preserves a local draft when a status refresh arrives", async () => {
    const instance = app();
    instance.wifiAuto = {
      adapters: [{ adapterGuid: "abc-123", interfaceName: "Wi-Fi", profiles: ["Office", "Backup"] }],
      rules: [{ adapterGuid: "abc-123", profileName: "Office", intervalSeconds: 60, enabled: false }],
      statuses: [],
    };
    const draft = instance.wifiAutoDraft("ABC-123", adapter);
    instance.updateWifiAutoDraft("ABC-123", { profileName: "Backup", intervalSeconds: 120 });
    api.wifiAutoStatus.mockResolvedValue({
      adapters: [{ adapterGuid: "abc-123", interfaceName: "Wi-Fi", profiles: ["Office", "Backup"] }],
      rules: [{ adapterGuid: "abc-123", profileName: "Office", intervalSeconds: 60, enabled: true }],
      statuses: [{ adapterGuid: "abc-123", state: "waiting" }],
    });

    await instance.loadWifiAutoStatus();

    expect(draft.profileName).toBe("Office");
    expect(instance.wifiAutoDraft("ABC-123", adapter)).toMatchObject({
      profileName: "Backup",
      intervalSeconds: 120,
    });
  });

  it("keeps a disabled local draft disabled after a successful disable", async () => {
    const instance = app();
    instance.wifiAutoDrafts.set("abc-123", {
      adapterGuid: "abc-123",
      profileName: "Office",
      intervalSeconds: 60,
      enabled: true,
    });
    instance.run = vi.fn((_key, task) => task());
    api.disableWifiAuto.mockResolvedValue({ disabled: true });
    api.wifiAutoStatus.mockResolvedValue({ adapters: [], rules: [], statuses: [] });

    await instance.disableWifiAuto("abc-123");

    expect(instance.wifiAutoDrafts.get("abc-123").enabled).toBe(false);
  });

  it("rejects intervals outside the supported range without previewing a write", async () => {
    const instance = app();
    await instance.saveWifiAuto({
      adapterGuid: "abc-123",
      interfaceName: "Wi-Fi",
      profileName: "Office",
      intervalSeconds: 4,
      enabled: true,
    });

    expect(api.previewWifiAuto).not.toHaveBeenCalled();
    expect(instance.message.text).toBe(instance.t("wifiAutoIntervalInvalid"));
  });

  it("previews and confirms a rule before saving it", async () => {
    const instance = app();
    instance.run = vi.fn((_key, task) => task());
    api.previewWifiAuto.mockResolvedValue({
      rule: { adapterGuid: "abc-123", profileName: "Office", intervalSeconds: 60, enabled: true },
      confirmationToken: "token",
      warnings: ["The rule can switch networks."],
    });
    api.saveWifiAuto.mockResolvedValue({ saved: true });
    api.wifiAutoStatus.mockResolvedValue({ adapters: [], rules: [], statuses: [] });

    await instance.saveWifiAuto({
      adapterGuid: "abc-123",
      interfaceName: "Wi-Fi",
      profileName: "Office",
      intervalSeconds: 60,
      enabled: true,
    });

    expect(api.saveWifiAuto).not.toHaveBeenCalled();
    expect(instance.confirmation).not.toBeNull();
    await instance.confirm();
    expect(api.saveWifiAuto).toHaveBeenCalledWith(
      expect.objectContaining({ profileName: "Office", enabled: true }),
      "token",
    );
  });

  it("disables a rule directly without opening confirmation", async () => {
    const instance = app();
    instance.run = vi.fn((_key, task) => task());
    api.disableWifiAuto.mockResolvedValue({ disabled: true });
    api.wifiAutoStatus.mockResolvedValue({ adapters: [], rules: [], statuses: [] });

    await instance.disableWifiAuto("abc-123");

    expect(api.disableWifiAuto).toHaveBeenCalledWith("abc-123");
    expect(instance.confirmation).toBeNull();
  });

  it("retains discovery errors and translates worker states", async () => {
    const instance = app();
    api.wifiAutoStatus.mockResolvedValue({
      adapters: [],
      rules: [],
      statuses: [],
      discoveryError: "netsh unavailable",
    });

    await instance.loadWifiAutoStatus();

    expect(instance.wifiAuto.discoveryError).toBe("netsh unavailable");
    expect(instance.wifiAutoStateText("development-disabled")).toBe(
      instance.t("wifiAutoDevelopmentDisabled"),
    );
    expect(instance.wifiAutoStateText("connecting")).toBe(
      instance.t("wifiAutoConnecting"),
    );
  });
});
