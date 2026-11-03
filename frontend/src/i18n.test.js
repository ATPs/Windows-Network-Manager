import { describe, expect, it } from "vitest";
import { copy, messages } from "./i18n.js";

describe("interface copy", () => {
  it("keeps every English key available in Chinese", () => {
    expect(Object.keys(copy["zh-CN"]).sort()).toEqual(
      Object.keys(copy.en).sort(),
    );
  });

  it("includes navigation and safety messages", () => {
    for (const key of [
      "overview",
      "adapters",
      "routes",
      "profiles",
      "diagnostics",
      "maintenance",
      "permissionHelp",
      "operationFailed",
    ]) {
      expect(copy.en[key]).toBeTruthy();
      expect(copy["zh-CN"][key]).toBeTruthy();
    }
  });

  it("translates the backend notices of network backups", () => {
    for (const code of [
      "backup.reason.ipSettings",
      "backup.reason.category",
      "backup.reason.metric",
      "backup.reason.route",
      "backup.reason.hosts",
      "backup.reason.trigger",
      "backup.reason.restore",
      "backup.reason.defaults",
      "backup.reason.backupRestore",
      "backup.created",
      "backup.renamed",
      "backup.kept",
      "backup.deleted",
      "restore.skip.missing",
      "restore.skip.protected",
      "restore.skip.system",
      "restore.skip.invalid",
      "restore.skip.unreadable",
      "restore.skip.domain",
      "restore.skip.routeAdapter",
      "restore.skip.hostsUnreadable",
      "restore.skip.hostsUnavailable",
      "restore.hostsOverwrite",
      "restore.notIncluded",
      "restore.backup.complete",
      "restore.backup.partial",
      "scope.backupRestore",
    ]) {
      expect(messages["zh-CN"][code], code).toBeTruthy();
    }
  });
});
