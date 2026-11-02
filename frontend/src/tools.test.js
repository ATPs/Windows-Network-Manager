import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  connectivityTest: vi.fn(),
  lookupRoute: vi.fn(),
  startTrace: vi.fn(),
  cancelTrace: vi.fn(),
  activeConnections: vi.fn(),
  scanWifi: vi.fn(),
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
let views;
let channelSummary;
let filterConnections;
beforeAll(async () => {
  vi.stubGlobal("customElements", { define() {} });
  vi.stubGlobal("localStorage", { getItem: () => null, setItem() {} });
  ({ NetworkManagerApp, views } = await import("./network-manager-app.js"));
  ({ channelSummary, filterConnections } = await import("./views/tools.js"));
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(() => vi.resetAllMocks());

function app() {
  const instance = new NetworkManagerApp();
  instance.querySelector = () => null;
  instance.refresh = vi.fn().mockResolvedValue(undefined);
  return instance;
}

const connections = [
  { protocol: "TCP", state: "Listen", localAddress: "127.0.0.1", localPort: 8889, process: "xray.exe", pid: 76936, product: "v2rayN" },
  { protocol: "TCP", state: "Established", localAddress: "192.168.1.20", localPort: 50000, remoteAddress: "1.1.1.1", remotePort: 443, process: "msedge.exe", pid: 10 },
  { protocol: "UDP", state: "", localAddress: "0.0.0.0", localPort: 11010, process: "easytier-gui.exe", pid: 3380, product: "EasyTier" },
];

describe("Tools page", () => {
  it("is reachable from navigation after Diagnostics", () => {
    expect(views.indexOf("tools")).toBe(views.indexOf("diagnostics") + 1);
    expect(app().t("tools")).toBe("Tools");
  });

  it("summarizes channel use per band, busiest first", () => {
    const summary = channelSummary([
      { band: "5 GHz", channel: 36 },
      { band: "2.4 GHz", channel: 6 },
      { band: "2.4 GHz", channel: 1 },
      { band: "2.4 GHz", channel: 6 },
      { band: null, channel: null },
    ]);
    expect(summary).toEqual([
      { band: "2.4 GHz", channels: [{ channel: 6, count: 2 }, { channel: 1, count: 1 }] },
      { band: "5 GHz", channels: [{ channel: 36, count: 1 }] },
    ]);
  });

  it("filters connections by text, protocol and state", () => {
    expect(filterConnections(connections, { search: "v2rayn" }).map((row) => row.pid)).toEqual([76936]);
    expect(filterConnections(connections, { search: "1.1.1.1" }).map((row) => row.pid)).toEqual([10]);
    expect(filterConnections(connections, { protocol: "UDP" })).toHaveLength(1);
    expect(filterConnections(connections, { state: "listening" }).map((row) => row.pid)).toEqual([76936, 3380]);
    expect(filterConnections(connections, { state: "established" }).map((row) => row.pid)).toEqual([10]);
  });

  it("runs tools only on request and keeps their results", async () => {
    const instance = app();
    api.connectivityTest.mockResolvedValue({ adapters: [], defaultPath: {} });
    api.lookupRoute.mockResolvedValue({ address: "1.1.1.1", route: { interfaceIndex: 23 } });
    await instance.lookupRoute();
    expect(api.lookupRoute).not.toHaveBeenCalled();
    instance.tools.lookupHost = " 1.1.1.1 ";
    await instance.lookupRoute();
    expect(api.lookupRoute).toHaveBeenCalledWith("1.1.1.1");
    expect(instance.tools.lookup.address).toBe("1.1.1.1");
    await instance.runConnectivityTest();
    expect(instance.tools.connectivity).toEqual({ adapters: [], defaultPath: {} });
  });

  it("streams trace hops for the current trace only", async () => {
    const instance = app();
    instance.tools.lookupHost = "1.1.1.1";
    api.startTrace.mockResolvedValue({ traceId: 2, target: "1.1.1.1", address: "1.1.1.1", maxHops: 30 });
    await instance.startTrace();
    expect(instance.isBusy("tools:trace")).toBe(true);
    instance.onTraceHop({ traceId: 1, hop: 1 });
    instance.onTraceHop({ traceId: 2, hop: 1, address: "192.168.1.1", rttsMs: [1, 1], status: "transit" });
    expect(instance.tools.trace.hops).toHaveLength(1);
    instance.onTraceDone({ traceId: 2, reached: true });
    expect(instance.tools.trace.reached).toBe(true);
    expect(instance.isBusy("tools:trace")).toBe(false);
  });
});

describe("Coexistence with tunnel and proxy software", () => {
  it("keeps protected adapters out of Restore Windows defaults", () => {
    const instance = app();
    instance.network.overview = {
      interfaces: [
        { interfaceIndex: 23, alias: "Ethernet", managedBy: null },
        { interfaceIndex: 33, alias: "et_6_0qmn", managedBy: "EasyTier" },
        { interfaceIndex: 13, alias: "vgate0", managedBy: "tunnel" },
      ],
    };
    expect(instance.defaultsCandidates().map((item) => item.alias)).toEqual(["Ethernet"]);
  });

  it("labels owners and stores the refresh summary", () => {
    const instance = app();
    expect(instance.managedLabel("EasyTier")).toBe("Managed by EasyTier");
    expect(instance.managedLabel("tunnel")).toBe("Managed by tunnel software");
    instance.applyNetwork({ overview: { interfaces: [] }, coexistence: { software: [{ product: "v2rayN" }] } });
    expect(instance.coexistence.software[0].product).toBe("v2rayN");
    instance.applyNetwork({ overview: { interfaces: [] } });
    expect(instance.coexistence.software[0].product).toBe("v2rayN");
  });
});
