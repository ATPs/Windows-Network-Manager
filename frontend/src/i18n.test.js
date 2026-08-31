import { describe, expect, it } from "vitest";
import { copy } from "./i18n.js";

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
});
