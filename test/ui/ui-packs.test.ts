import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { changeScope, toStoredRules, type StoredRule } from "../../ui/poll.js";
import { data, ui } from "../../ui/state.js";
import { catalogKinds, catalogPacks, packSwitch, pageRules, sheetRules, shownRules, sortPageRules, switchTurnsOn } from "../../ui/pack-model.js";
import { packSwitchWrites, settingWrites } from "../../ui/settings-writes.js";

type Request = { method: string; body: unknown; answer: (status: number, body?: unknown) => void };

const packStart = { by: "pack-start", packId: "typescript" };
const byRepository = { by: "repository", folder: "", target: "rule" };

/** One entry of `GET /api/repository/rules`: a rule present through `source` at `folder`. */
const entry = (id: string, folder: string, fields: Record<string, unknown> = {}) => ({
  kind: "file",
  rule: { id, mode: "steer", clauses: [] },
  folder,
  on: true,
  mode: "steer",
  source: { kind: "pack", id: "typescript" },
  onSource: packStart,
  modeSource: packStart,
  suppressions: "honour",
  suppressionsSource: packStart,
  config: null,
  configSource: {},
  revision: 1,
  ...fields,
});

const pack = (id: string, name: string, category: string, ruleIds: string[] = []) => ({
  id, name, category, covers: `${name} rules.`,
  rules: ruleIds.map(rid => ({ id: rid, title: `Title of ${rid}`, group: "Group", mode: "steer", on: true })),
});

const stored = (id: string) => (data.rules as Map<string, StoredRule>).get(id)!;
const ui2 = ui as unknown as { filter: string; query: string; sort: string };

afterEach(() => {
  vi.unstubAllGlobals();
  data.rules = new Map();
  data.attachedPacks = new Map();
  data.packs = [];
  data.activity = {};
  Object.assign(ui2, { filter: "all", query: "", sort: "violations" });
});

describe("the whole-pack switch", () => {
  const rules = (...on: boolean[]) => on.map((enabled, i) => ({ rule: { id: `r${i}` }, enabled }));

  it("is on only when every rule is on: some on, none on and no rules all read off", () => {
    expect(packSwitch(rules(true, false, true))).toEqual({ on: 2, total: 3, state: "false" });
    expect(packSwitch(rules(true, true))).toEqual({ on: 2, total: 2, state: "true" });
    expect(packSwitch(rules(false, false))).toEqual({ on: 0, total: 2, state: "false" });
    expect(packSwitch([])).toEqual({ on: 0, total: 0, state: "false" });
  });

  it("off with some rules on turns the rest on, then on turns everything off, then on again", () => {
    let set = rules(true, false, false);
    const click = () => { const on = switchTurnsOn(packSwitch(set).state); set = set.map(r => ({ ...r, enabled: on })); };

    click();
    expect(packSwitch(set).state).toBe("true");
    click();
    expect(packSwitch(set).state).toBe("false");
    click();
    expect(packSwitch(set).state).toBe("true");
  });
});

describe("the writes the whole-pack switch sends", () => {
  let requests: Request[] = [];
  const puts = () => requests.filter(r => r.method === "PUT");
  const toast = vi.fn();
  const page = (owner: "repository" | "checkout") => settingWrites({ toast, invalidate: () => {}, owner: () => owner });

  beforeEach(() => {
    requests = [];
    toast.mockReset();
    vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
    vi.stubGlobal("fetch", (_input: URL, init: { method?: string; body?: string } = {}) => {
      const { promise, resolve } = Promise.withResolvers<Response>();
      requests.push({
        method: init.method ?? "GET",
        body: init.body === undefined ? undefined : JSON.parse(init.body),
        answer: (status, body) => resolve(new Response(body === undefined ? null : JSON.stringify(body), { status })),
      });
      return promise;
    });
    data.repository = null;
    changeScope({ repository: "A", checkout: "/work/a" });
    data.rules = toStoredRules([
      entry("ts:a", ""),
      // Off as the pack starts it.
      entry("ts:b", "", { on: false }),
      // Switched on with a setting of its own, which outranks the pack's at one owner and folder.
      entry("ts:c", "", { onSource: byRepository }),
      entry("ts:c", "web"),
      // The same pack at a second folder.
      entry("ts:d", "web", { on: false }),
      // Added on its own, not through the pack: the pack's setting does not decide it.
      entry("ts:own", "", { source: { kind: "rule", id: "ts:own" } }),
    ]);
    data.attachedPacks = new Map([["typescript", ["", "web"]]]);
  });

  it("writes the pack's on at each folder it is added at, then clears its rules' own on there", () => {
    expect(packSwitchWrites("typescript", true)).toEqual([
      { folder: "", targets: [{ kind: "pack", id: "typescript" }], setting: { on: true } },
      { folder: "", targets: [{ kind: "rule", id: "ts:a" }, { kind: "rule", id: "ts:b" }, { kind: "rule", id: "ts:c" }], setting: { on: null } },
      { folder: "web", targets: [{ kind: "pack", id: "typescript" }], setting: { on: true } },
      { folder: "web", targets: [{ kind: "rule", id: "ts:c" }, { kind: "rule", id: "ts:d" }], setting: { on: null } },
    ]);
  });

  it("with some rules on turns every rule of the pack on, owned as the page says, and shows it at once", async () => {
    const before = packSwitch(([...data.rules.values()] as StoredRule[]).filter(r => r.source.kind === "pack"));
    expect(before).toMatchObject({ state: "false", on: 2 });

    const saving = page("checkout").setPackEnabled("typescript", switchTurnsOn(before.state), "On.");

    expect(["ts:a", "ts:b", "ts:c", "ts:d"].map(id => stored(id).enabled)).toEqual([true, true, true, true]);
    expect(stored("ts:own").enabled).toBe(true);
    for (let i = 0; i < 4; i += 1) {
      await vi.waitFor(() => expect(puts()).toHaveLength(i + 1));
      puts()[i]!.answer(200, { settings: [] });
    }
    await saving;
    expect(puts().map(r => r.body)).toEqual([
      { repository: "A", checkout: "/work/a", folder: "", owner: "checkout", targets: [{ kind: "pack", id: "typescript" }], on: true },
      { repository: "A", checkout: "/work/a", folder: "", owner: "checkout", targets: [{ kind: "rule", id: "ts:a" }, { kind: "rule", id: "ts:b" }, { kind: "rule", id: "ts:c" }], on: null },
      { repository: "A", checkout: "/work/a", folder: "web", owner: "checkout", targets: [{ kind: "pack", id: "typescript" }], on: true },
      { repository: "A", checkout: "/work/a", folder: "web", owner: "checkout", targets: [{ kind: "rule", id: "ts:c" }, { kind: "rule", id: "ts:d" }], on: null },
    ]);
    expect(toast).toHaveBeenCalledWith("On.");
  });

  it("from all on turns them all off, rules switched on by their own setting included", async () => {
    for (const r of data.rules.values()) (r as StoredRule).enabled = true;
    const state = packSwitch([...data.rules.values()] as StoredRule[]).state;
    expect(state).toBe("true");

    const saving = page("repository").setPackEnabled("typescript", switchTurnsOn(state), "Off.");

    expect(stored("ts:c").enabled).toBe(false);
    for (let i = 0; i < 4; i += 1) {
      await vi.waitFor(() => expect(puts()).toHaveLength(i + 1));
      puts()[i]!.answer(200, { settings: [] });
    }
    await saving;
    const bodies = puts().map(r => r.body as { owner: string; on: boolean | null; targets: Array<{ kind: string }> });
    expect(bodies.map(b => [b.owner, b.targets[0]!.kind, b.on])).toEqual([
      ["repository", "pack", false], ["repository", "rule", null], ["repository", "pack", false], ["repository", "rule", null],
    ]);
  });

  it("from all off turns them all on", () => {
    for (const r of data.rules.values()) (r as StoredRule).enabled = false;
    const state = packSwitch([...data.rules.values()] as StoredRule[]).state;

    expect(switchTurnsOn(state)).toBe(true);
    expect(packSwitchWrites("typescript", true).filter(w => w.targets[0]!.kind === "pack").map(w => w.setting.on)).toEqual([true, true]);
  });

  it("skips a folder still loading for an add, and writes nothing for a pack not added", () => {
    data.attachedPacks = new Map<string, string[]>([["typescript", ["", Promise.resolve("web") as unknown as string]]]);

    expect(packSwitchWrites("typescript", false).map(w => w.folder)).toEqual(["", ""]);
    expect(packSwitchWrites("python", true)).toEqual([]);
  });

  it("puts the switches back when the first write fails", async () => {
    const saving = page("checkout").setPackEnabled("typescript", true, "On.");

    await vi.waitFor(() => expect(puts()).toHaveLength(1));
    puts()[0]!.answer(500, { error: "boom", message: "boom" });
    await saving;

    expect(stored("ts:b").enabled).toBe(false);
    expect(stored("ts:a").enabled).toBe(true);
    expect(puts()).toHaveLength(1);
    expect(toast).toHaveBeenCalledWith(expect.stringMatching(/^Could not save/u));
  });
});

describe("a pack page's rules", () => {
  beforeEach(() => {
    data.packs = [pack("typescript", "TypeScript", "language", ["ts:a", "ts:b", "ts:c"]), pack("next", "Next.js", "framework", ["next:a"])];
    data.rules = toStoredRules([
      entry("ts:a", "", { addedAt: 100 }),
      entry("ts:b", "", { on: false, addedAt: 300 }),
      entry("ts:c", "", { addedAt: null }),
      entry("next:a", "", { source: { kind: "pack", id: "next" } }),
      entry("mine:x", "", { source: { kind: "library_rule", id: "mine:x" } }),
    ]);
    data.activity = { "ts:a": { measured: true, distinctViolations: 2, lastSeenAt: 10 }, "ts:c": { measured: true, distinctViolations: 5, lastSeenAt: 5 } };
  });

  it("lists the pack's rules only, Created by you's own, and nothing for a pack not added", () => {
    expect(pageRules("pack:typescript")!.map(r => r.rule.id).sort()).toEqual(["ts:a", "ts:b", "ts:c"]);
    expect(pageRules("mine")!.map(r => r.rule.id)).toEqual(["mine:x"]);
    expect(pageRules("pack:python")).toBeNull();
  });

  it("sorts by violations, latest violation, newest added (undated last) and title", () => {
    const all = pageRules("pack:typescript")!;
    const ids = (sort: string) => sortPageRules(all, sort).map(r => r.rule.id);

    expect(ids("violations")).toEqual(["ts:c", "ts:a", "ts:b"]);
    expect(ids("recent")).toEqual(["ts:a", "ts:c", "ts:b"]);
    expect(ids("newest")).toEqual(["ts:b", "ts:a", "ts:c"]);
    expect(ids("az")).toEqual(["ts:a", "ts:b", "ts:c"]);
  });

  it("filters by on, off, violations and the search", () => {
    const all = pageRules("pack:typescript")!;
    const shown = (filter: string, query = "") => { Object.assign(ui2, { filter, query, sort: "az" }); return shownRules(all).map(r => r.rule.id); };

    expect(shown("on")).toEqual(["ts:a", "ts:c"]);
    expect(shown("off")).toEqual(["ts:b"]);
    expect(shown("fired")).toEqual(["ts:a", "ts:c"]);
    expect(shown("all", "ts:b")).toEqual(["ts:b"]);
    expect(shown("on", "ts:b")).toEqual([]);
  });
});

describe("the catalog", () => {
  const packs = [
    pack("vue", "vue", "framework", ["v"]), pack("python", "Python", "language", ["p"]),
    pack("next", "Next.js", "framework", ["n"]), pack("blade", "Blade", "language", ["b"]),
  ];

  it("offers All and the kinds the packs have, languages before frameworks", () => {
    expect(catalogKinds(packs)).toEqual([["all", "All"], ["language", "Language"], ["framework", "Framework"]]);
    expect(catalogKinds(packs.filter(p => p.category === "framework"))).toEqual([["all", "All"], ["framework", "Framework"]]);
  });

  it("lists one kind or every pack, alphabetical whatever the case", () => {
    expect(catalogPacks(packs, "all").map(p => p.id)).toEqual(["blade", "next", "python", "vue"]);
    expect(catalogPacks(packs, "framework").map(p => p.id)).toEqual(["next", "vue"]);
    expect(catalogPacks(packs, "language").map(p => p.id)).toEqual(["blade", "python"]);
  });

  it("filters a pack sheet's rules by title or id, A to Z", () => {
    const p = { ...pack("x", "X", "language"), rules: [{ id: "x:2", title: "Beta" }, { id: "x:1", title: "Alpha" }, { id: "x:3", title: "Gamma" }] };

    expect(sheetRules(p, "").map(r => r.id)).toEqual(["x:1", "x:2", "x:3"]);
    expect(sheetRules(p, " ETA ").map(r => r.id)).toEqual(["x:2"]);
    expect(sheetRules(p, "x:3").map(r => r.id)).toEqual(["x:3"]);
  });
});
