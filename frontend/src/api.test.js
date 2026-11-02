import { beforeEach, describe, expect, it, vi } from "vitest";

const invoke = vi.hoisted(() => vi.fn().mockResolvedValue({ ok: true }));
vi.mock("@tauri-apps/api/core", () => ({ invoke }));

const { api } = await import("./api.js");

describe("Wi-Fi auto-connect API", () => {
  beforeEach(() => invoke.mockClear());

  it("uses explicit Tauri commands and preserves the rule payload", async () => {
    const rule = {
      adapterGuid: "abc-123",
      profileName: "Office",
      intervalSeconds: 60,
      enabled: true,
    };
    await api.wifiAutoStatus();
    await api.previewWifiAuto(rule);
    await api.saveWifiAuto(rule, "token");
    await api.disableWifiAuto("abc-123");
    await api.removeWifiAuto("abc-123");

    expect(invoke.mock.calls).toEqual([
      ["get_wifi_auto_connect_status", {}],
      ["preview_wifi_auto_connect", { input: rule }],
      ["save_wifi_auto_connect", { input: { rule, confirmationToken: "token" } }],
      ["disable_wifi_auto_connect", { adapterGuid: "abc-123" }],
      ["remove_wifi_auto_connect", { adapterGuid: "abc-123" }],
    ]);
  });
});
