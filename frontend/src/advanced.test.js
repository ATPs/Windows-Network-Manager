import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  hostsFile: vi.fn(),
  previewHostsChange: vi.fn(),
  applyHostsChange: vi.fn(),
  previewConnectNewWifi: vi.fn(),
  connectNewWifi: vi.fn(),
  previewNetworkTrigger: vi.fn(),
  saveNetworkTrigger: vi.fn(),
  previewDomainRoute: vi.fn(),
  applyDomainRoute: vi.fn(),
  previewVpnAction: vi.fn(),
  applyVpnAction: vi.fn(),
  vpnConnections: vi.fn(),
  profiles: vi.fn(),
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

/** Dotted IPv4 text for test data. */
const ip = (...parts) => parts.join(".");

let NetworkManagerApp;
let views;
let hostsView;
beforeAll(async () => {
  vi.stubGlobal("customElements", { define() {} });
  vi.stubGlobal("localStorage", { getItem: () => null, setItem() {} });
  vi.stubGlobal("document", { hidden: false, documentElement: { dataset: {} } });
  ({ NetworkManagerApp, views } = await import("./network-manager-app.js"));
  hostsView = await import("./views/hosts.js");
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(() => {
  vi.resetAllMocks();
  api.profiles.mockResolvedValue({});
  api.vpnConnections.mockResolvedValue({ connections: [] });
});

function app() {
  const instance = new NetworkManagerApp();
  instance.querySelector = () => null;
  instance.refresh = vi.fn().mockResolvedValue(undefined);
  instance.privilege = { isAdmin: true };
  return instance;
}

describe("Hosts file", () => {
  it("is a page before Recovery", () => {
    expect(views.indexOf("hosts")).toBe(views.indexOf("maintenance") - 1);
  });

  it("validates host names and addresses like the backend", () => {
    expect(hostsView.parseHostNames("nas.lan, nas")).toEqual({ names: ["nas.lan", "nas"], valid: true });
    expect(hostsView.parseHostNames("*.ads.example").valid).toBe(false);
    expect(hostsView.parseHostNames("").valid).toBe(false);
    expect(hostsView.isHostsAddress(ip(192, 168, 1, 10))).toBe(true);
    expect(hostsView.isHostsAddress("::1")).toBe(true);
    expect(hostsView.isHostsAddress("nas")).toBe(false);
  });

  it("tracks edits and saves only after preview and acknowledgement", async () => {
    const instance = app();
    const lines = [
      { kind: "raw", text: "# comment" },
      { kind: "entry", ip: "::1", names: ["localhost"], comment: "", enabled: true, original: "::1 localhost" },
    ];
    api.hostsFile.mockResolvedValue({ lines, hash: "h1", path: "hosts" });
    await instance.loadHosts();
    expect(instance.hostsDirty()).toBe(false);
    instance.addHostsLine({ ip: ip(192, 168, 1, 10), names: ["nas.lan"] });
    instance.updateHostsLine(1, { enabled: false });
    expect(instance.hostsDirty()).toBe(true);
    api.previewHostsChange.mockResolvedValue({ diff: { added: ["nas"], removed: [] }, confirmationToken: "t", notices: [] });
    api.applyHostsChange.mockResolvedValue({ backup: "hosts-1.bak" });
    await instance.saveHosts();
    const input = api.previewHostsChange.mock.calls[0][0];
    expect(input.baseHash).toBe("h1");
    expect(input.lines).toHaveLength(3);
    expect(input.lines.every((line) => !("key" in line))).toBe(true);
    await instance.confirm();
    expect(api.applyHostsChange).not.toHaveBeenCalled();
    instance.confirmation.acknowledged = true;
    await instance.confirm();
    expect(api.applyHostsChange).toHaveBeenCalledWith(input, "t");
  });
});

describe("Connect to a new Wi-Fi network", () => {
  it("offers only unsaved networks with supported security", () => {
    const instance = app();
    expect(instance.canConnectNew({ ssid: "Cafe", security: "WPA2-Personal" })).toBe(true);
    expect(instance.canConnectNew({ ssid: "Cafe", security: "WPA2-Enterprise" })).toBe(false);
    expect(instance.canConnectNew({ ssid: "Home", security: "WPA2-Personal", savedProfile: true })).toBe(false);
    expect(instance.canConnectNew({ ssid: "", security: "Open" })).toBe(false);
  });

  it("clears the password from UI state before sending it once", async () => {
    const instance = app();
    instance.tools.wifiScan = { adapterGuid: "guid" };
    const network = { adapterGuid: "guid", ssid: "Cafe", security: "WPA2-Personal" };
    api.previewConnectNewWifi.mockResolvedValue({ network, confirmationToken: "t", interfaceName: "WLAN", notices: [] });
    api.connectNewWifi.mockImplementation(async () => {
      expect(instance.newWifi).toBeNull();
      return { message: "sent" };
    });
    await instance.startConnectNew({ ssid: "Cafe", security: "WPA2-Personal" });
    instance.newWifi.password = "correct horse";
    await instance.submitNewWifi();
    expect(api.connectNewWifi).toHaveBeenCalledWith({ ...network, confirmationToken: "t" }, "correct horse");
    expect(JSON.stringify(instance.activity)).not.toContain("correct horse");
  });
});

describe("Automatic configurations, fallback networks, domain routes and VPN", () => {
  it("needs acknowledgement before saving a trigger", async () => {
    const instance = app();
    const draft = { adapterGuid: "guid", profileName: "Office", savedConfigId: "cfg", enabled: true };
    api.previewNetworkTrigger.mockResolvedValue({ trigger: { ...draft, id: "t1" }, confirmationToken: "t", notices: [] });
    api.saveNetworkTrigger.mockResolvedValue({});
    await instance.saveTrigger(draft);
    expect(instance.confirmation.acknowledge).toBeTruthy();
    instance.confirmation.acknowledged = true;
    await instance.confirm();
    expect(api.saveNetworkTrigger).toHaveBeenCalledWith({ ...draft, id: "t1" }, "t");
  });

  it("drops duplicate and primary fallback networks", () => {
    const instance = app();
    const rule = instance.wifiAutoRuleInput({
      adapterGuid: "g",
      profileName: "Office",
      intervalSeconds: 60,
      enabled: true,
      fallbackProfiles: ["Home", "Office", "Home", "", "Phone"],
    });
    expect(rule.fallbackProfiles).toEqual(["Home", "Phone"]);
  });

  it("shows the planned domain routes and applies the previewed plan", async () => {
    const instance = app();
    const gateway = ip(192, 168, 1, 1);
    const plan = {
      domain: "example.com",
      add: [{ destinationPrefix: `${ip(93, 184, 216, 34)}/32`, nextHop: gateway }],
      remove: [],
    };
    api.previewDomainRoute.mockResolvedValue({ plan, unchanged: false, confirmationToken: "t", notices: [] });
    api.applyDomainRoute.mockResolvedValue({ results: [] });
    await instance.domainRoute({ domain: "example.com", nextHop: gateway, interfaceIndex: 23, routeMetric: 5 });
    expect(api.previewDomainRoute).toHaveBeenCalledWith(expect.objectContaining({ mode: "save" }));
    expect(instance.confirmation.rows[0].kind).toBe("add");
    await instance.confirm();
    expect(api.applyDomainRoute).toHaveBeenCalledWith(plan, "t");
  });

  it("treats VPN disconnect as destructive", async () => {
    const instance = app();
    api.previewVpnAction.mockResolvedValue({ confirmationToken: "t", notices: [] });
    await instance.vpnAction({ name: "Office" }, "disconnect");
    expect(instance.confirmation.destructive).toBe(true);
    await instance.vpnAction({ name: "Office" }, "connect");
    expect(instance.confirmation.destructive).toBe(false);
  });
});
