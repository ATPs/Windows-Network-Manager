import { afterAll, beforeAll, describe, expect, it, vi } from "vitest";

// Every page is rendered once so that a missing import or helper fails here
// instead of leaving a blank page in the app. The template tag is mocked, but
// its arguments (the nested helpers) are still evaluated.
vi.mock("./api.js", () => ({ api: new Proxy({}, { get: () => vi.fn().mockResolvedValue({}) }) }));
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
beforeAll(async () => {
  vi.stubGlobal("customElements", { define() {} });
  vi.stubGlobal("localStorage", { getItem: () => null, setItem() {} });
  vi.stubGlobal("document", { hidden: false, documentElement: { dataset: {} } });
  ({ NetworkManagerApp, views } = await import("./network-manager-app.js"));
});
afterAll(() => vi.unstubAllGlobals());

function populated(instance) {
  const gateway = ip(192, 168, 1, 1);
  instance.network.overview = {
    interfaces: [
      {
        interfaceIndex: 12,
        alias: "Ethernet",
        description: "Test adapter",
        status: "Up",
        ipv4Addresses: [ip(192, 168, 1, 20)],
        metrics: { ipv4: { interfaceMetric: 25, automaticMetric: true }, ipv6: { interfaceMetric: 25, automaticMetric: true } },
      },
    ],
  };
  instance.network.routes = {
    routes: [
      {
        addressFamily: "IPv4",
        destinationPrefix: `${ip(0, 0, 0, 0)}/0`,
        nextHop: gateway,
        interfaceIndex: 12,
        interfaceAlias: "Ethernet",
        routeMetric: 0,
        interfaceMetric: 25,
        effectiveMetric: 25,
        defaultRoute: true,
        removable: true,
      },
    ],
  };
  const guid = "255bc7a7-a80c-47e6-902b-386aa6ee90db";
  const now = Date.now();
  instance.monitor.snapshot = {
    traffic: {
      total: [[now - 2000, 100, 50], [now, 200, 80]],
      interfaces: [{ key: guid, alias: "Ethernet", hardware: true, up: true, rxBps: 200, txBps: 80, samples: [[now - 2000, 100, 50], [now, 200, 80]] }],
    },
    signal: [{ adapterGuid: "w", alias: "Wi-Fi", ssid: "Home", signalPercent: 80, samples: [[now - 10000, 70, "Home"], [now, 80, "Home"]] }],
    latency: [{ kind: "gateway", address: gateway, alias: "Ethernet", samples: [[now - 10000, 2], [now, null]] }],
    latencyEnabled: true,
  };
  instance.monitor.selected = [guid];
  instance.monitor.storage = { path: "C:\Apps\monitor-data", source: "install", sizeBytes: 2048, days: 2, fallbackReason: "installUnwritable" };
  instance.backups = {
    backups: [
      { id: "m", name: "Home", kind: "manual", createdAt: new Date(now).toISOString(), adapters: [{ alias: "Ethernet", target: { ipv4: { mode: "dhcp" }, dns: { mode: "automatic" } }, metrics: { ipv4: { automaticMetric: true } }, networkCategory: "Private" }], routes: [], hosts: { hash: "h", entries: 3 } },
      { id: "a", name: "", kind: "automatic", reason: { code: "backup.reason.hosts", params: {}, text: "Before editing the hosts file" }, createdAt: new Date(now).toISOString(), adapters: [], routes: [], hosts: null },
    ],
    limits: { manual: 50, automatic: 20 },
  };
  instance.backupDetails = new Set(["m"]);
  instance.config.domainRoutes = [
    { domain: "example.com", nextHop: gateway, interfaceIndex: 12, routeMetric: 5, addresses: [ip(93, 184, 216, 34)] },
  ];
}

describe("page rendering", () => {
  for (const isAdmin of [false, true]) {
    for (const withData of [false, true]) {
      it(`renders every page (${isAdmin ? "admin" : "standard"}, ${withData ? "data" : "loading"})`, () => {
        const instance = new NetworkManagerApp();
        instance.querySelector = () => null;
        instance.privilege = { isAdmin };
        if (withData) populated(instance);
        for (const view of views) {
          instance.view = view;
          expect(() => instance.render(), view).not.toThrow();
        }
      });
    }
  }

  it("renders Monitor history ranges and the automatic backup list", () => {
    const instance = new NetworkManagerApp();
    instance.querySelector = () => null;
    instance.privilege = { isAdmin: true };
    populated(instance);
    const guid = instance.monitor.selected[0];
    const point = (t) => [t, 100, 50, 300, 90];
    instance.monitor.range = "7d";
    instance.monitor.history = {
      bucketSeconds: 1800,
      available: [{ key: guid, alias: "Ethernet", owner: null, hardware: true }],
      total: { points: [point(1), [2, null, null, null, null]], rxBytes: 10, txBytes: 5, rxAvg: 1, txAvg: 1, rxPeak: 3, txPeak: 2 },
      interfaces: [{ key: guid, alias: "Ethernet", points: [point(1), point(2)], rxBytes: 10, txBytes: 5, rxPeak: 3, uptimePct: 99 }],
      latency: [{ kind: "dns", address: "x", alias: "Ethernet", points: [[1, 3, 4, 0], [2, null, null, null]], stats: { sent: 2, received: 1, avg: 3, p95: 3, max: 3, lossPct: 50 } }],
      wifi: [{ adapterGuid: "w", alias: "Wi-Fi", ssids: ["Home"], points: [[1, 70, 60], [2, 80, 75]] }],
      daily: [{ day: "2026-11-03", rxBytes: 10, txBytes: 5, disconnects: 1, routeChanges: 0, wifiChanges: 2 }],
      events: { down: 1, route: 0 },
    };
    instance.view = "monitor";
    expect(() => instance.render()).not.toThrow();
    instance.monitor.cards = { traffic: false, latency: false, signal: false, daily: false, events: false, notifications: false };
    expect(() => instance.render()).not.toThrow();
    instance.backupTab = "automatic";
    instance.view = "maintenance";
    expect(() => instance.render()).not.toThrow();
  });

  it("renders drawers and dialogs", async () => {
    const instance = new NetworkManagerApp();
    instance.querySelector = () => null;
    instance.privilege = { isAdmin: true };
    populated(instance);
    const states = [
      () => Object.assign(instance, { routeDrawerOpen: true, editingRoute: null }),
      () => Object.assign(instance, { routeDrawerOpen: true, editingRoute: instance.network.routes.routes[0] }),
      () => Object.assign(instance, { routeDrawerOpen: false, activityOpen: true }),
      () => instance.requestConfirmation(["line"], () => {}, true, {
        rows: [{ kind: "add", text: "row" }],
        acknowledge: "I understand",
        typeToConfirm: "Office",
      }),
      () => Object.assign(instance, { confirmation: null, nameDialog: { kind: "profile", value: "" } }),
      () => Object.assign(instance, { nameDialog: { kind: "backup", value: "" } }),
      () => Object.assign(instance, { nameDialog: { kind: "renameBackup", value: "Home", id: "m" } }),
      () => Object.assign(instance, { nameDialog: null, pendingRevert: { revertId: "r", deadline: Date.now() + 60000, label: "IP" } }),
      () => Object.assign(instance, {
        pendingRevert: null,
        newWifi: { network: { ssid: "Cafe", security: "WPA2-Personal" }, preview: { notices: [] }, password: "", show: false },
      }),
    ];
    for (const [index, apply] of states.entries()) {
      apply();
      expect(() => instance.render(), `state ${index}`).not.toThrow();
    }
    await instance.openIpEditor(instance.network.overview.interfaces[0]);
    expect(instance.ipEditor?.draft).toBeTruthy();
    expect(() => instance.render(), "ip editor").not.toThrow();
  });

  it("names the one adapter in power confirmations", async () => {
    const instance = new NetworkManagerApp();
    instance.querySelector = () => null;
    instance.privilege = { isAdmin: true };
    await instance.adapterAction({ interfaceIndex: 7, alias: "WLAN" }, "disable");
    expect(instance.confirmation.title).toBe("Disable WLAN?");
    expect(instance.confirmation.confirmLabel).toBe("Disable WLAN");
    expect(instance.confirmation.lines).toContain("Only WLAN is affected; other adapters keep working.");
  });
});
