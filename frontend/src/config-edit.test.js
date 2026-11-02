import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  ipSettings: vi.fn(),
  previewIpSettings: vi.fn(),
  applyIpSettings: vi.fn(),
  previewNetworkCategory: vi.fn(),
  applyNetworkCategory: vi.fn(),
  keepConfigChange: vi.fn(),
  revertConfigChange: vi.fn(),
  wifiProfileDetails: vi.fn(),
  previewWifiProfileChange: vi.fn(),
  applyWifiProfileChange: vi.fn(),
  profiles: vi.fn(),
  wifiProfiles: vi.fn(),
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
let editor;
let format;
beforeAll(async () => {
  vi.stubGlobal("customElements", { define() {} });
  vi.stubGlobal("localStorage", { getItem: () => null, setItem() {} });
  vi.stubGlobal("document", { hidden: false, documentElement: { dataset: {} } });
  ({ NetworkManagerApp } = await import("./network-manager-app.js"));
  editor = await import("./views/ip-editor.js");
  format = await import("./format.js");
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(() => {
  vi.resetAllMocks();
  api.profiles.mockResolvedValue({});
});

function app() {
  const instance = new NetworkManagerApp();
  instance.querySelector = () => null;
  instance.refresh = vi.fn().mockResolvedValue(undefined);
  instance.privilege = { isAdmin: true };
  return instance;
}

const ethernet = { interfaceIndex: 23, alias: "Ethernet", guid: "{AAAA}", metrics: { ipv4: {} }, networkCategory: "Public" };

describe("Extra confirmation steps", () => {
  it("requires the acknowledgement before confirming", async () => {
    const instance = app();
    const action = vi.fn();
    instance.requestConfirmation(["x"], action, true, { acknowledge: "I understand" });
    await instance.confirm();
    expect(action).not.toHaveBeenCalled();
    expect(instance.confirmation).not.toBeNull();
    instance.confirmation.acknowledged = true;
    await instance.confirm();
    expect(action).toHaveBeenCalledOnce();
  });

  it("requires typing the exact name to forget a network", async () => {
    const instance = app();
    const action = vi.fn();
    instance.requestConfirmation(["x"], action, true, { typeToConfirm: "Home WiFi" });
    instance.confirmation.typed = "home wifi";
    await instance.confirm();
    expect(action).not.toHaveBeenCalled();
    instance.confirmation.typed = " Home WiFi ";
    await instance.confirm();
    expect(action).toHaveBeenCalledOnce();
  });
});

describe("IP settings", () => {
  it("previews, needs acknowledgement, applies and starts the keep countdown", async () => {
    vi.useFakeTimers();
    const instance = app();
    api.ipSettings.mockResolvedValue({ target: { ipv4: { mode: "dhcp" }, dns: { mode: "automatic" } } });
    await instance.openIpEditor(ethernet);
    expect(instance.ipEditor.draft.ipv4Mode).toBe("dhcp");
    instance.updateIpDraft({ ipv4Mode: "static", address: ip(192, 168, 1, 50), prefixLength: "24", gateway: ip(192, 168, 1, 1) });
    expect(instance.ipEditor.draft.dnsMode).toBe("manual");
    instance.updateIpDraft({ dnsServers: `${ip(1, 1, 1, 1)}, ${ip(8, 8, 8, 8)}` });
    const request = instance.ipRequest(ethernet, instance.ipEditor.draft);
    expect(request.dns.servers).toEqual([ip(1, 1, 1, 1), ip(8, 8, 8, 8)]);
    api.previewIpSettings.mockResolvedValue({
      settings: {
        unchanged: false,
        target: { ipv4: request.ipv4, dns: request.dns },
        before: { ipv4: { mode: "dhcp" }, dns: { mode: "automatic" } },
      },
      notices: [{ code: "ip.interrupt", text: "Ethernet briefly loses its address." }],
      confirmationToken: "t",
    });
    api.applyIpSettings.mockResolvedValue({ revert: { revertId: 7, seconds: 60 }, message: "IP settings updated." });
    await instance.saveIpSettings();
    expect(api.applyIpSettings).not.toHaveBeenCalled();
    expect(instance.confirmation.rows.map((row) => row.before)).toEqual(["Automatic (DHCP)", "Automatic"]);
    await instance.confirm();
    expect(api.applyIpSettings).not.toHaveBeenCalled();
    instance.confirmation.acknowledged = true;
    await instance.confirm();
    expect(api.applyIpSettings).toHaveBeenCalledWith(request, "t");
    expect(instance.ipEditor).toBeNull();
    expect(instance.pendingRevert.revertId).toBe(7);
    expect(instance.revertSecondsLeft()).toBe(60);
    vi.advanceTimersByTime(15_000);
    expect(instance.revertSecondsLeft()).toBe(45);
    api.keepConfigChange.mockResolvedValue({ kept: true });
    await instance.keepChange();
    expect(api.keepConfigChange).toHaveBeenCalledWith(7);
    expect(instance.pendingRevert).toBeNull();
    vi.useRealTimers();
  });

  it("reverts on request and clears the countdown when the backend reverts", async () => {
    const instance = app();
    instance.startRevertCountdown({ revertId: 3, seconds: 60 }, "IP");
    api.revertConfigChange.mockResolvedValue({ reverted: true });
    await instance.revertChange();
    expect(api.revertConfigChange).toHaveBeenCalledWith(3);
    expect(instance.pendingRevert).toBeNull();
    instance.startRevertCountdown({ revertId: 4, seconds: 60 }, "IP");
    instance.onConfigReverted({ revertId: 4, automatic: true });
    expect(instance.pendingRevert).toBeNull();
    expect(instance.toasts.at(-1).kind).toBe("warning");
  });

  it("only offers the editor for adapters the app may change", () => {
    const instance = app();
    expect(instance.canEditIp(ethernet)).toBe(true);
    expect(instance.canEditIp({ ...ethernet, managedBy: "EasyTier" })).toBe(false);
    expect(instance.canEditIp({ ...ethernet, systemInterface: true })).toBe(false);
  });

  it("validates drafts and shows the subnet mask", () => {
    const instance = app();
    expect(editor.prefixToMask(24)).toBe(ip(255, 255, 255, 0));
    expect(editor.prefixToMask(20)).toBe(ip(255, 255, 240, 0));
    expect(editor.prefixToMask(33)).toBe("");
    const draft = { ipv4Mode: "static", address: "192.168.1", prefixLength: "40", gateway: "x", dnsMode: "manual", dnsServers: "" };
    expect(Object.keys(editor.validateIpDraft(instance, draft)).sort()).toEqual(["address", "dnsServers", "gateway", "prefixLength"]);
    const good = { ipv4Mode: "dhcp", dnsMode: "manual", dnsServers: `${ip(1, 1, 1, 1)}, 2606:4700:4700::1111` };
    expect(editor.validateIpDraft(instance, good)).toEqual({});
  });
});

describe("Network category and saved networks", () => {
  it("requires acknowledgement for a category change", async () => {
    const instance = app();
    api.previewNetworkCategory.mockResolvedValue({ unchanged: false, category: { before: "Public" }, confirmationToken: "t" });
    api.applyNetworkCategory.mockResolvedValue({ revert: { revertId: 9, seconds: 60 } });
    await instance.changeNetworkCategory(ethernet, "Private");
    expect(instance.confirmation.rows[0]).toMatchObject({ before: "Public network", after: "Private network" });
    expect(instance.confirmation.acknowledge).toBeTruthy();
    instance.confirmation.acknowledged = true;
    await instance.confirm();
    expect(api.applyNetworkCategory).toHaveBeenCalledWith({ interfaceIndex: 23, category: "Private" }, "t");
    expect(instance.pendingRevert.revertId).toBe(9);
  });

  it("asks to type the profile name before forgetting it", async () => {
    const instance = app();
    api.previewWifiProfileChange.mockResolvedValue({ confirmationToken: "t", notices: [] });
    await instance.wifiProfileAction(ethernet, { name: "Office", editable: true }, "forget");
    expect(instance.confirmation.typeToConfirm).toBe("Office");
    expect(instance.confirmation.destructive).toBe(true);
    await instance.wifiProfileAction(ethernet, { name: "Office", editable: true }, "moveUp");
    expect(instance.confirmation.typeToConfirm).toBe("");
  });
});

describe("IPv6 routes", () => {
  it("validates IPv6 literals and prefixes", () => {
    expect(format.isIpv6("fe80::1")).toBe(true);
    expect(format.isIpv6("2001:db8:0:0:0:0:0:1")).toBe(true);
    expect(format.isIpv6("2001:db8::1::2")).toBe(false);
    expect(format.isIpv6(ip(10, 0, 0, 1))).toBe(false);
    expect(format.isIpv6Cidr("2001:db8::/32")).toBe(true);
    expect(format.isIpv6Cidr("2001:db8::/129")).toBe(false);
  });

  it("opens the route form in the family of the edited route", () => {
    const instance = app();
    instance.openRouteDrawer({ addressFamily: "IPv6", destinationPrefix: "2001:db8::/32", nextHop: "fe80::1", interfaceIndex: 5, routeMetric: 5 });
    expect(instance.routeDraftFamily).toBe("IPv6");
    instance.closeRouteDrawer();
    instance.routeFamily = "IPv4";
    instance.openRouteDrawer();
    expect(instance.routeDraftFamily).toBe("IPv4");
  });
});
