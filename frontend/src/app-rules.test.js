import { afterAll, beforeAll, beforeEach, describe, expect, it, vi } from "vitest";

const api = vi.hoisted(() => ({
  appRules: vi.fn(),
  saveAppRule: vi.fn(),
  removeAppRule: vi.fn(),
  pickProgram: vi.fn(),
  runningPrograms: vi.fn(),
  checkLocalProxy: vi.fn(),
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
let views;
let appsView;
beforeAll(async () => {
  vi.stubGlobal("customElements", { define() {} });
  vi.stubGlobal("localStorage", { getItem: () => null, setItem() {} });
  vi.stubGlobal("document", { hidden: false, documentElement: { dataset: {} } });
  ({ NetworkManagerApp, views } = await import("./network-manager-app.js"));
  appsView = await import("./views/apps.js");
});
afterAll(() => vi.unstubAllGlobals());
beforeEach(() => {
  vi.resetAllMocks();
  api.profiles.mockResolvedValue({});
  api.appRules.mockResolvedValue({ rules: [], export: { rulesJson: "[]", ruleCount: 0, servers: [], usesFolders: false } });
});

function app() {
  const instance = new NetworkManagerApp();
  instance.querySelector = () => null;
  instance.refresh = vi.fn().mockResolvedValue(undefined);
  instance.refreshConfig = vi.fn().mockResolvedValue(undefined);
  return instance;
}

describe("per-app rules", () => {
  it("is a page after Tools", () => {
    expect(views.indexOf("apps")).toBe(views.indexOf("tools") + 1);
  });

  it("normalizes program entries like the backend", () => {
    const normalize = appsView.normalizeProgramEntry;
    expect(normalize(" chrome.exe ")).toBe("chrome.exe");
    expect(normalize("c:\\Apps\\Tool\\tool.exe")).toBe("C:/Apps/Tool/tool.exe");
    expect(normalize("D:\\Apps\\Tool\\")).toBe("D:/Apps/Tool/");
    for (const bad of ["", "chrome", "*.exe", "Apps\\tool.exe", "C:\\Apps\\..\\tool.exe", "C:\\readme.txt", "C:\\", "\\\\server\\a.exe"]) {
      expect(normalize(bad), bad).toBe("");
    }
  });

  it("validates drafts", () => {
    const instance = app();
    instance.newAppRule();
    const draft = instance.appRuleDraft;
    expect(appsView.appRuleErrors(draft)).toEqual({ name: true, programs: true });
    instance.updateAppRuleDraft({ name: "Dev", target: "local" });
    instance.addAppRuleProgram("git.exe");
    expect(appsView.appRuleErrors(instance.appRuleDraft)).toEqual({ port: true });
    instance.updateAppRuleDraft({ localProxy: { protocol: "socks", host: "example.com", port: "8889" } });
    expect(appsView.appRuleErrors(instance.appRuleDraft)).toEqual({ host: true });
    instance.updateAppRuleDraft({ localProxy: { protocol: "socks", host: "localhost", port: "8889" } });
    expect(appsView.appRuleErrors(instance.appRuleDraft)).toEqual({});
  });

  it("takes the exact case of a running program and turns a path into its folder", async () => {
    const instance = app();
    api.runningPrograms.mockResolvedValue({ programs: [{ name: "Teams.exe", path: "C:/Apps/Teams/Teams.exe" }] });
    await instance.loadRunningPrograms();
    instance.newAppRule();
    instance.addAppRuleProgram("teams.exe");
    instance.addAppRuleProgram("C:\\Apps\\Tool\\tool.exe");
    instance.addAppRuleProgram("Teams.exe");
    expect(instance.appRuleDraft.programs).toEqual(["Teams.exe", "C:/Apps/Tool/tool.exe"]);
    instance.useProgramFolder("C:/Apps/Tool/tool.exe");
    expect(instance.appRuleDraft.programs).toEqual(["Teams.exe", "C:/Apps/Tool/"]);
    expect(instance.addAppRuleProgram("not a program")).toBe(false);
  });

  it("saves only the rule fields and reloads the v2rayN setup", async () => {
    const instance = app();
    instance.newAppRule();
    instance.updateAppRuleDraft({ name: " Dev ", target: "local", localProxy: { protocol: "http", host: "localhost", port: "8080" } });
    instance.addAppRuleProgram("git.exe");
    api.saveAppRule.mockResolvedValue({ rules: [] });
    await instance.saveAppRule();
    const saved = api.saveAppRule.mock.calls[0][0];
    expect(saved).toEqual({
      id: saved.id,
      name: "Dev",
      programs: ["git.exe"],
      target: "local",
      enabled: true,
      localProxy: { protocol: "http", host: "localhost", port: 8080 },
    });
    expect(saved.id).toMatch(/^rule-[a-z0-9-]+$/);
    expect(instance.appRuleDraft).toBeNull();
    expect(api.appRules).toHaveBeenCalledWith(false);
  });

  it("drops the local proxy for other targets and asks before removing", async () => {
    const instance = app();
    instance.editAppRule({ id: "r1", name: "Web", programs: ["chrome.exe"], target: "local", localProxy: { protocol: "socks", host: "localhost", port: 1080 }, enabled: true });
    instance.updateAppRuleDraft({ target: "direct" });
    expect(instance.appRuleInput(instance.appRuleDraft).localProxy).toBeUndefined();
    api.removeAppRule.mockResolvedValue({ removed: true });
    instance.removeAppRule({ id: "r1", name: "Web", programs: ["chrome.exe"] });
    expect(api.removeAppRule).not.toHaveBeenCalled();
    expect(instance.confirmation.destructive).toBe(true);
    await instance.confirm();
    expect(api.removeAppRule).toHaveBeenCalledWith("r1");
  });

  it("renders rules, the editor, a port warning and the v2rayN steps", () => {
    const instance = app();
    instance.view = "apps";
    instance.appRules = {
      rules: [{ id: "r1", name: "Dev", programs: ["git.exe", "C:/Tools/"], target: "local", localProxy: { protocol: "socks", host: "localhost", port: 8889 }, enabled: true }],
      export: {
        rulesJson: "[]",
        ruleCount: 1,
        servers: [
          { remark: "WNM-socks-localhost-8889", protocol: "socks", host: "localhost", port: 8889, link: "socks://localhost:8889#WNM-socks-localhost-8889" },
          { remark: "WNM-http-localhost-8080", protocol: "http", host: "localhost", port: 8080, link: null },
        ],
        usesFolders: true,
      },
    };
    instance.runningPrograms = [{ name: "git.exe", path: "C:/Git/git.exe" }];
    instance.editAppRule(instance.appRules.rules[0]);
    instance.portCheck = { port: 8889, listening: true, loop: true, process: "xray", message: "loop" };
    expect(() => instance.render()).not.toThrow();
  });
});
