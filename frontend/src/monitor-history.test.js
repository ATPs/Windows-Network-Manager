import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  monitorSnapshot: vi.fn(),
  networkEvents: vi.fn(),
  monitorHistory: vi.fn(),
  monitorStorage: vi.fn(),
  updateSettings: vi.fn(),
  chooseMonitorFolder: vi.fn(),
  setMonitorFolder: vi.fn(),
  clearMonitorHistory: vi.fn(),
  profiles: vi.fn(),
}));

vi.mock("./api.js", () => ({ api }));
vi.mock("lit", () => ({
  LitElement: class {
    requestUpdate() {}
    get updateComplete() { return Promise.resolve(); }
  },
  html: (strings) => ({ strings }),
  svg: () => undefined,
  nothing: undefined,
}));
vi.mock("lit/directives/keyed.js", () => ({ keyed: (_key, value) => value }));

const storage = new Map();
let NetworkManagerApp;
let format;
let charts;
let monitorView;
beforeAll(async () => {
  vi.stubGlobal("customElements", { define() {} });
  vi.stubGlobal("localStorage", { getItem: (key) => storage.get(key) ?? null, setItem: (key, value) => storage.set(key, value) });
  vi.stubGlobal("document", { hidden: false, documentElement: { dataset: {} } });
  ({ NetworkManagerApp } = await import("./network-manager-app.js"));
  format = await import("./format.js");
  charts = await import("./charts.js");
  monitorView = await import("./views/monitor.js");
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(() => {
  vi.resetAllMocks();
  storage.clear();
  api.profiles.mockResolvedValue({});
});

function app() {
  const instance = new NetworkManagerApp();
  instance.querySelector = () => null;
  return instance;
}

const GUID = "255bc7a7-a80c-47e6-902b-386aa6ee90db";

describe("monitor formatting helpers", () => {
  it("formats bytes, latency and byte rates", () => {
    expect(format.formatBytes(0)).toBe("0 B");
    expect(format.formatBytes(1536)).toBe("1.5 KB");
    expect(format.formatBytes(5 * 1024 ** 3)).toBe("5.0 GB");
    expect(format.formatMs(12.4)).toBe("12 ms");
    expect(format.formatMs(null)).toBe("—");
    expect(charts.formatByteRate(2048)).toBe("2.0 KB/s");
  });

  it("summarizes rates and latency", () => {
    const rates = format.summarizeRates([[0, 100], [2000, 300], [4000, null], [6000, 200]]);
    expect(rates.peak).toBe(300);
    expect(rates.avg).toBe(200);
    expect(rates.total).toBe(300 * 2 + 200 * 2);
    const latency = format.summarizeLatency([2, 4, null, 10]);
    expect(latency).toMatchObject({ sent: 4, max: 10, p95: 10, lossPct: 25 });
    expect(latency.avg).toBeCloseTo(16 / 3);
    expect(format.summarizeLatency([]).lossPct).toBeNull();
  });

  it("draws daily bars only when there is data", () => {
    expect(charts.barChart({ items: [], formatValue: String, ariaLabel: "x" })).toBeUndefined();
    expect(charts.barChart({ items: [{ label: "11-03", values: [1, 2] }], formatValue: String, ariaLabel: "x" })).toBeTruthy();
  });
});

describe("monitor ranges and interfaces", () => {
  it("loads saved history for a range with the selected interfaces and remembers the choice", async () => {
    api.monitorHistory.mockResolvedValue({ range: "24h", total: { points: [] }, available: [] });
    const instance = app();
    instance.view = "monitor";
    instance.toggleMonitorInterface(GUID);
    instance.setMonitorRange("24h");
    await vi.waitFor(() => expect(api.monitorHistory).toHaveBeenCalledWith({ range: "24h", interfaces: [GUID] }));
    expect(JSON.parse(storage.get("wnm-monitor-prefs"))).toMatchObject({ range: "24h", selected: [GUID] });
    // A new window starts on the remembered range.
    expect(app().monitor.range).toBe("24h");
    instance.stopMonitorPolling();
  });

  it("reloads history once a minute only while a history range is shown", () => {
    vi.useFakeTimers();
    api.monitorSnapshot.mockResolvedValue({ traffic: { total: [] } });
    api.networkEvents.mockResolvedValue({ events: [] });
    api.monitorHistory.mockResolvedValue({});
    const instance = app();
    instance.setView("monitor");
    vi.advanceTimersByTime(120_000);
    expect(api.monitorHistory).not.toHaveBeenCalled();
    instance.setMonitorRange("7d");
    vi.advanceTimersByTime(120_500);
    expect(api.monitorHistory).toHaveBeenCalledTimes(3);
    instance.setView("tools");
    vi.advanceTimersByTime(300_000);
    expect(api.monitorHistory).toHaveBeenCalledTimes(3);
    vi.useRealTimers();
  });

  it("offers live and previously seen interfaces in the picker", () => {
    const list = monitorView.monitorInterfaces(
      { traffic: { interfaces: [{ key: "a", alias: "Ethernet", hardware: true, up: true }] } },
      { available: [{ key: "a", alias: "Ethernet" }, { key: "b", alias: "Old Wi-Fi" }] },
    );
    expect(list.map((item) => item.alias)).toEqual(["Ethernet", "Old Wi-Fi"]);
  });

  it("computes the stat tiles from saved history", () => {
    const instance = app();
    instance.monitor.range = "24h";
    instance.monitor.history = {
      total: { rxAvg: 100, txAvg: 50, rxPeak: 900, txPeak: 300, rxBytes: 1000, txBytes: 500 },
      latency: [
        { kind: "gateway", stats: { sent: 10, received: 9, avg: 3, p95: 5, max: 8 } },
        { kind: "dns", stats: { sent: 10, received: 10, avg: 12, p95: 20, max: 30 } },
      ],
      events: { down: 2, route: 1 },
    };
    const stats = monitorView.monitorStats(instance);
    expect(stats).toMatchObject({ avgRx: 100, peakTx: 300, totalRx: 1000, disconnects: 2, routeChanges: 1 });
    expect(stats.gateway).toMatchObject({ avg: 3, lossPct: 10 });
    expect(stats.dns.avg).toBe(12);
  });

  it("switches between bit and byte rates", () => {
    const instance = app();
    expect(instance.formatRate(125_000)).toBe("1.0 Mbps");
    instance.setRateUnit("bytes");
    expect(instance.formatRate(2048)).toBe("2.0 KB/s");
  });
});

describe("monitor history settings", () => {
  it("defaults to 30 days of history with local latency checks", () => {
    const monitor = app().settings.monitor;
    expect(monitor).toMatchObject({ historyEnabled: true, retentionDays: 30, latencyEnabled: true, latencyPublicDns: false });
  });

  it("asks before keeping less history", async () => {
    api.updateSettings.mockImplementation(async (settings) => settings);
    api.monitorStorage.mockResolvedValue({ path: "x", days: 3 });
    const instance = app();
    await instance.updateMonitorSettings({ retentionDays: 7 });
    expect(api.updateSettings).not.toHaveBeenCalled();
    expect(instance.confirmation.destructive).toBe(true);
    await instance.confirm();
    expect(api.updateSettings).toHaveBeenCalledWith(expect.objectContaining({ monitor: expect.objectContaining({ retentionDays: 7 }) }));
    await instance.updateMonitorSettings({ retentionDays: 90 });
    expect(api.updateSettings).toHaveBeenCalledTimes(2);
  });

  it("changes the folder only after confirming the folder Rust picked", async () => {
    api.chooseMonitorFolder.mockResolvedValue({ candidate: { path: "D:\\Logs\\monitor-data", days: 0 }, confirmationToken: "t1" });
    api.setMonitorFolder.mockResolvedValue({ settings: { monitor: { folder: "D:\\Logs" } }, storage: { path: "D:\\Logs\\monitor-data" } });
    const instance = app();
    await instance.chooseMonitorFolder();
    expect(api.setMonitorFolder).not.toHaveBeenCalled();
    expect(instance.confirmation.lines).toContain("D:\\Logs\\monitor-data");
    await instance.confirm();
    expect(api.setMonitorFolder).toHaveBeenCalledWith("t1", true);
    expect(instance.monitor.storage.path).toBe("D:\\Logs\\monitor-data");
  });

  it("does nothing when the folder picker is cancelled", async () => {
    api.chooseMonitorFolder.mockResolvedValue({ cancelled: true });
    const instance = app();
    await instance.chooseMonitorFolder();
    expect(instance.confirmation).toBeNull();
  });
});

describe("text size", () => {
  it("steps through the sizes and back to default", () => {
    const instance = app();
    instance.updateSettings = vi.fn((patch) => Object.assign(instance.settings, patch));
    instance.stepFontSize(1);
    expect(instance.settings.fontSize).toBe("large");
    instance.stepFontSize(1);
    instance.stepFontSize(1);
    expect(instance.settings.fontSize).toBe("xlarge");
    instance.stepFontSize(-1);
    instance.stepFontSize(-1);
    instance.stepFontSize(-1);
    instance.stepFontSize(-1);
    expect(instance.settings.fontSize).toBe("small");
    instance.stepFontSize(0);
    expect(instance.settings.fontSize).toBe("default");
  });

  it("uses Ctrl+=, Ctrl+- and Ctrl+0", () => {
    const instance = app();
    instance.updateSettings = vi.fn((patch) => Object.assign(instance.settings, patch));
    const press = (key) => instance.handleShortcut({ key, ctrlKey: true, altKey: false, preventDefault() {} });
    press("=");
    expect(instance.settings.fontSize).toBe("large");
    press("0");
    expect(instance.settings.fontSize).toBe("default");
    press("-");
    expect(instance.settings.fontSize).toBe("small");
  });
});
