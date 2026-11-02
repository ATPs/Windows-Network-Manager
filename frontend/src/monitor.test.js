import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  monitorSnapshot: vi.fn(),
  networkEvents: vi.fn(),
  clearNetworkEvents: vi.fn(),
  updateSettings: vi.fn(),
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
let views;
let charts;
let filterEvents;
beforeAll(async () => {
  vi.stubGlobal("customElements", { define() {} });
  vi.stubGlobal("localStorage", { getItem: () => null, setItem() {} });
  ({ NetworkManagerApp, views } = await import("./network-manager-app.js"));
  charts = await import("./charts.js");
  ({ filterEvents } = await import("./views/monitor.js"));
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(() => vi.resetAllMocks());

function app() {
  const instance = new NetworkManagerApp();
  instance.querySelector = () => null;
  return instance;
}

describe("chart helpers", () => {
  it("rounds axis maxima and formats bit rates", () => {
    expect(charts.niceMax(0)).toBe(1);
    expect(charts.niceMax(730)).toBe(1000);
    expect(charts.niceMax(1200)).toBe(2000);
    expect(charts.niceMax(4100)).toBe(5000);
    expect(charts.formatRate(0)).toBe("0 bps");
    expect(charts.formatRate(125)).toBe("1.0 Kbps");
    expect(charts.formatRate(1_500_000)).toBe("12.0 Mbps");
    expect(charts.formatRate(200_000_000)).toBe("1.6 Gbps");
  });

  it("draws gaps for missing samples and clamps to the maximum", () => {
    const box = { start: 0, end: 10, max: 100, width: 100, height: 50 };
    const path = charts.linePath([[0, 0], [5, null], [10, 200]], box);
    expect(path.match(/M/g)).toHaveLength(2);
    expect(path).toContain("M52.0,58.0");
    expect(path).toContain("M152.0,8.0");
  });
});

describe("Monitor page", () => {
  it("follows Tools in the navigation", () => {
    expect(views.indexOf("monitor")).toBe(views.indexOf("tools") + 1);
    expect(app().t("monitor")).toBe("Monitor");
  });

  it("hides tunnel up/down events unless requested", () => {
    const events = [
      { kind: "adapter", tunnel: true },
      { kind: "wifi", tunnel: false },
      { kind: "defaultRoute", tunnel: false },
    ];
    expect(filterEvents(events)).toHaveLength(2);
    expect(filterEvents(events, { tunnels: true })).toHaveLength(3);
    expect(filterEvents(events, { kind: "wifi" })).toEqual([{ kind: "wifi", tunnel: false }]);
  });

  it("merges live events newest first once the history is loaded", () => {
    const instance = app();
    instance.onNetworkEvents([{ id: "1" }]);
    expect(instance.monitor.events).toBeNull();
    instance.monitor.events = [{ id: "0" }];
    instance.onNetworkEvents([{ id: "1" }, { id: "2" }]);
    expect(instance.monitor.events.map((event) => event.id)).toEqual(["2", "1", "0"]);
  });

  it("polls only while the Monitor page is open", () => {
    vi.useFakeTimers();
    vi.stubGlobal("document", { hidden: false });
    api.monitorSnapshot.mockResolvedValue({ traffic: { total: [] } });
    api.networkEvents.mockResolvedValue({ events: [] });
    const instance = app();
    instance.setView("overview");
    expect(api.monitorSnapshot).not.toHaveBeenCalled();
    instance.setView("monitor");
    vi.advanceTimersByTime(4100);
    expect(api.monitorSnapshot).toHaveBeenCalledTimes(3);
    instance.setView("tools");
    vi.advanceTimersByTime(10_000);
    expect(api.monitorSnapshot).toHaveBeenCalledTimes(3);
    vi.useRealTimers();
  });

  it("keeps notifications off by default and saves changes", async () => {
    vi.stubGlobal("document", { hidden: false, documentElement: { dataset: {} } });
    const instance = app();
    expect(instance.settings.notifications.enabled).toBe(false);
    api.updateSettings.mockImplementation(async (settings) => settings);
    await instance.updateNotifications({ enabled: true });
    expect(api.updateSettings).toHaveBeenCalledWith(
      expect.objectContaining({ notifications: expect.objectContaining({ enabled: true, tunnels: false }) }),
    );
    expect(instance.settings.notifications.enabled).toBe(true);
  });
});
