import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  backups: vi.fn(),
  createBackup: vi.fn(),
  renameBackup: vi.fn(),
  keepBackup: vi.fn(),
  deleteBackup: vi.fn(),
  previewBackupRestore: vi.fn(),
  applyBackupRestore: vi.fn(),
  refresh: vi.fn(),
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

let NetworkManagerApp;
beforeAll(async () => {
  vi.stubGlobal("customElements", { define() {} });
  vi.stubGlobal("localStorage", { getItem: () => null, setItem() {} });
  vi.stubGlobal("document", { hidden: false, documentElement: { dataset: {} } });
  ({ NetworkManagerApp } = await import("./network-manager-app.js"));
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(() => {
  vi.resetAllMocks();
  api.backups.mockResolvedValue({ backups: [], limits: { manual: 50, automatic: 20 } });
  api.refresh.mockResolvedValue({});
  api.profiles.mockResolvedValue({});
});

function app() {
  const instance = new NetworkManagerApp();
  instance.querySelector = () => null;
  instance.privilege = { isAdmin: true };
  return instance;
}

const backup = { id: "b1", name: "Home", kind: "manual", createdAt: "2026-11-03T08:00:00Z", adapters: [], routes: [] };

describe("network backups", () => {
  it("loads backups when Recovery opens", async () => {
    const instance = app();
    instance.setView("maintenance");
    await vi.waitFor(() => expect(api.backups).toHaveBeenCalled());
  });

  it("creates a backup with an optional name", async () => {
    api.createBackup.mockResolvedValue({ backup });
    const instance = app();
    instance.openNameDialog("backup");
    vi.stubGlobal("FormData", class { get() { return ""; } });
    await instance.saveNamed({ preventDefault() {}, currentTarget: {} });
    expect(api.createBackup).toHaveBeenCalledWith("");
    expect(api.backups).toHaveBeenCalled();
  });

  it("renames a backup from the name dialog", async () => {
    api.renameBackup.mockResolvedValue({ renamed: true });
    const instance = app();
    instance.openNameDialog("renameBackup", "Home", "b1");
    expect(instance.nameDialog).toMatchObject({ kind: "renameBackup", value: "Home", id: "b1" });
    vi.stubGlobal("FormData", class { get() { return " Office "; } });
    await instance.saveNamed({ preventDefault() {}, currentTarget: {} });
    expect(api.renameBackup).toHaveBeenCalledWith("b1", "Office");
  });

  it("previews, needs an acknowledgement, applies and starts the keep-or-revert countdown", async () => {
    api.previewBackupRestore.mockResolvedValue({
      changes: [
        { kind: "ip", alias: "Ethernet", interfaceIndex: 7, target: { ipv4: { mode: "dhcp" }, dns: null }, before: { ipv4: { mode: "static", address: "x", prefixLength: 24 } } },
        { kind: "category", alias: "Ethernet", before: "Public", after: "Private" },
        { kind: "hosts" },
      ],
      skipped: [{ alias: "EasyTier", code: "restore.skip.protected", text: "EasyTier is managed by EasyTier." }],
      warnings: ["Choose Keep within 60 seconds."],
      confirmationToken: "tok",
    });
    api.applyBackupRestore.mockResolvedValue({ revert: { revertId: 5, seconds: 60 }, message: "ok" });
    const instance = app();
    instance.setBackupInclude("hosts", false);
    await instance.restoreBackup(backup);
    expect(api.previewBackupRestore).toHaveBeenCalledWith("b1", expect.objectContaining({ hosts: false, ip: true }));
    const confirmation = instance.confirmation;
    expect(confirmation.acknowledge).toBeTruthy();
    expect(confirmation.rows.map((row) => row.label)).toEqual(["Ethernet · IPv4", "Ethernet · Network category", "Hosts file", "EasyTier"]);
    expect(confirmation.rows.at(-1).kind).toBe("skip");
    expect(instance.confirmationReady()).toBe(false);
    confirmation.acknowledged = true;
    await instance.confirm();
    expect(api.applyBackupRestore).toHaveBeenCalledWith("b1", expect.objectContaining({ hosts: false }), "tok");
    expect(instance.pendingRevert?.revertId).toBe(5);
    instance.clearPendingRevert();
  });

  it("says so when nothing differs", async () => {
    api.previewBackupRestore.mockResolvedValue({ changes: [], skipped: [] });
    const instance = app();
    instance.notify = vi.fn();
    await instance.restoreBackup(backup);
    expect(instance.confirmation).toBeNull();
    expect(instance.notify).toHaveBeenCalled();
  });

  it("confirms before deleting and can keep automatic backups", async () => {
    api.deleteBackup.mockResolvedValue({ deleted: true });
    api.keepBackup.mockResolvedValue({ kept: true });
    const instance = app();
    instance.deleteBackup(backup);
    expect(api.deleteBackup).not.toHaveBeenCalled();
    expect(instance.confirmation.destructive).toBe(true);
    await instance.confirm();
    expect(api.deleteBackup).toHaveBeenCalledWith("b1");
    await instance.keepBackup({ ...backup, kind: "automatic" });
    expect(api.keepBackup).toHaveBeenCalledWith("b1");
  });

  it("labels unnamed automatic backups by their reason", () => {
    const instance = app();
    expect(instance.backupLabel({ name: "", reason: { code: "backup.reason.hosts", params: {}, text: "Before editing the hosts file" } })).toBe(
      "Before editing the hosts file",
    );
    expect(instance.backupLabel({ name: "", reason: null })).toBe("Backup");
  });
});
