import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  previewEditRoute: vi.fn(),
  editRoute: vi.fn(),
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
  vi.stubGlobal("localStorage", { getItem: () => null });
  vi.stubGlobal("FormData", class {
    constructor(values) { this.values = values; }
    get(key) { return this.values[key] ?? null; }
  });
  ({ NetworkManagerApp } = await import("./network-manager-app.js"));
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(() => vi.resetAllMocks());

const original = {
  addressFamily: "IPv4", destinationPrefix: "172.16.0.0/12",
  nextHop: "192.168.3.1", interfaceIndex: 26, routeMetric: 5,
  effectiveMetric: 40, persistent: true,
};
const replacement = { ...original, destinationPrefix: "172.18.0.0/16", routeMetric: 9 };
const event = () => ({
  preventDefault: vi.fn(),
  currentTarget: { ...replacement, persistent: "on" },
});
function app() {
  const instance = new NetworkManagerApp();
  instance.privilege.isAdmin = true;
  instance.querySelector = () => null;
  instance.refresh = vi.fn().mockResolvedValue(undefined);
  instance.refreshConfig = vi.fn().mockResolvedValue(undefined);
  return instance;
}

describe("route editing", () => {
  it("copies the selected route without substituting the effective metric", async () => {
    const instance = app();
    await instance.startRouteEdit(original);
    expect(instance.editingRoute).toEqual(original);
    expect(instance.editingRoute).not.toBe(original);
    expect(instance.editingRoute.routeMetric).toBe(5);
  });

  it("waits for confirmation and applies the server's canonical before/after values", async () => {
    const instance = app();
    await instance.startRouteEdit(original);
    const canonicalOld = { ...original, policyStores: ["ActiveStore", "PersistentStore"] };
    api.previewEditRoute.mockResolvedValue({ oldRoute: canonicalOld, route: replacement, confirmationToken: "token" });
    api.editRoute.mockResolvedValue({ edited: true });
    await instance.addRoute(event());
    expect(api.previewEditRoute).toHaveBeenCalledWith(original, expect.objectContaining({ routeMetric: 9 }));
    expect(api.editRoute).not.toHaveBeenCalled();
    expect(instance.confirmation.lines[0]).toContain("172.16.0.0/12");
    expect(instance.confirmation.lines[1]).toContain("172.18.0.0/16");
    await instance.confirm();
    expect(api.editRoute).toHaveBeenCalledWith(canonicalOld, replacement, "token");
    expect(instance.editingRoute).toBeNull();
    expect(instance.refresh).toHaveBeenCalled();
  });

  it("keeps the editor open and shows a failed save", async () => {
    const instance = app();
    await instance.startRouteEdit(original);
    api.previewEditRoute.mockResolvedValue({ oldRoute: original, route: replacement, confirmationToken: "token" });
    api.editRoute.mockRejectedValue({ message: "The original route was restored." });
    await instance.addRoute(event());
    await instance.confirm();
    expect(instance.editingRoute).toEqual(original);
    expect(instance.message).toEqual({ text: "The original route was restored.", kind: "error" });
  });

  it("does not request confirmation or write when the preview is unchanged", async () => {
    const instance = app();
    await instance.startRouteEdit(original);
    api.previewEditRoute.mockResolvedValue({ unchanged: true });
    await instance.addRoute(event());
    expect(instance.confirmation).toBeNull();
    expect(api.editRoute).not.toHaveBeenCalled();
  });
});
