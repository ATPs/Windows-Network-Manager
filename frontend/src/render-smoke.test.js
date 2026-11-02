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
