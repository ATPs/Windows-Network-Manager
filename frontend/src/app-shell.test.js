import { afterAll, afterEach, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  deleteProfile: vi.fn(),
  deleteRecent: vi.fn(),
  previewMetric: vi.fn(),
  profilePreview: vi.fn(),
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
let messages;
beforeAll(async () => {
  vi.stubGlobal("customElements", { define() {} });
  vi.stubGlobal("localStorage", { getItem: () => null, setItem() {} });
  ({ NetworkManagerApp } = await import("./network-manager-app.js"));
  ({ messages } = await import("./i18n.js"));
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(() => vi.resetAllMocks());
afterEach(() => vi.useRealTimers());

function app() {
  const instance = new NetworkManagerApp();
  instance.privilege.isAdmin = true;
  instance.querySelector = () => null;
  instance.refresh = vi.fn().mockResolvedValue(undefined);
  instance.refreshConfig = vi.fn().mockResolvedValue(undefined);
  return instance;
}

describe("feedback", () => {
  it("auto-dismisses success toasts but keeps errors", () => {
    vi.useFakeTimers();
    const instance = app();
    instance.notify("Saved", "success");
    instance.notify("Broken", "error");
    expect(instance.toasts).toHaveLength(2);
    vi.advanceTimersByTime(5000);
    expect(instance.toasts.map((toast) => toast.text)).toEqual(["Broken"]);
    expect(instance.message).toEqual({ text: "Broken", kind: "error" });
  });

  it("records write operations in the activity log", async () => {
    const instance = app();
    await instance.run("route:add", async () => ({ results: [{ status: "failed" }] }), [], "Add route");
    await instance.run("route:remove:x", async () => { throw new Error("denied"); }, []);
    expect(instance.activity.map((entry) => [entry.label, entry.status])).toEqual([
      [instance.t("actRouteRemove"), "failed"],
      ["Add route", "partial"],
    ]);
  });

  it("does not show success toasts or activity for read-only previews", async () => {
    const instance = app();
    const result = await instance.preview("metric:1:IPv4", async () => ({ ok: true }));
    expect(result).toEqual({ ok: true });
    expect(instance.toasts).toHaveLength(0);
    expect(instance.activity).toHaveLength(0);
  });
});

describe("backups", () => {
  it("asks for confirmation before deleting a saved configuration", async () => {
    const instance = app();
    api.deleteProfile.mockResolvedValue({ deleted: true });
    instance.deleteProfile({ id: "p1", name: "Office" });
    expect(api.deleteProfile).not.toHaveBeenCalled();
    expect(instance.confirmation.lines[0]).toBe("Office");
    await instance.confirm();
    expect(api.deleteProfile).toHaveBeenCalledWith("p1");
  });

  it("shows the restore diff returned by the preview", async () => {
    const instance = app();
    api.profilePreview.mockResolvedValue({
      confirmationToken: "token",
      summary: "Restore",
      unchangedCount: 2,
      changes: [
        {
          kind: "metric",
          alias: "Wi-Fi",
          addressFamily: "IPv4",
          before: { automatic: true, metric: 35 },
          after: { automatic: false, metric: 10 },
        },
        { kind: "routeAdd", route: { destinationPrefix: "10.0.0.0/8", nextHop: "192.168.1.1", interfaceAlias: "Ethernet" } },
      ],
    });
    await instance.restoreProfile({ id: "p1", name: "Office" });
    const rows = instance.confirmation.rows;
    expect(rows[0]).toMatchObject({ before: `${instance.t("automaticMetric")} (35)`, after: "10" });
    expect(rows[1]).toMatchObject({ kind: "add", after: expect.stringContaining("10.0.0.0/8") });
    expect(instance.confirmation.note).toContain("2");
  });
});

describe("routes and messages", () => {
  it("filters static routes and marks app-managed routes", () => {
    const instance = app();
    instance.routeFamily = "all";
    instance.network.routes = {
      routes: [
        { addressFamily: "IPv4", destinationPrefix: "10.0.0.0/8", nextHop: "192.168.1.1", interfaceIndex: 5, persistent: true },
        { addressFamily: "IPv4", destinationPrefix: "0.0.0.0/0", nextHop: "192.168.1.1", interfaceIndex: 5, persistent: false, protocol: "DHCP" },
      ],
    };
    instance.config.managedRoutes = [{ destinationPrefix: "10.0.0.0/8", nextHop: "192.168.1.1", interfaceIndex: 5 }];
    instance.routeStaticOnly = true;
    expect(instance.routeRows().map((route) => route.destinationPrefix)).toEqual(["10.0.0.0/8"]);
    expect(instance.managedRouteKeys().has("IPv4|10.0.0.0/8|192.168.1.1|5")).toBe(true);
  });

  it("translates backend notice codes and falls back to the English text", () => {
    const instance = app();
    messages["zh-CN"]["test.code"] = "删除 {destination}";
    instance.settings.language = "zh-CN";
    expect(instance.msg({ code: "test.code", params: { destination: "10.0.0.0/8" }, text: "Remove" })).toBe("删除 10.0.0.0/8");
    expect(instance.msg({ code: "unknown.code", text: "English fallback" })).toBe("English fallback");
    expect(instance.warningsOf({ warnings: ["plain"] })).toEqual(["plain"]);
    delete messages["zh-CN"]["test.code"];
  });
});
