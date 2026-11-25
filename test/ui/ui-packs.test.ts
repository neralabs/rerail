import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { changeScope, toStoredRules, type StoredRule } from "../../ui/poll.js";
import { data, ui } from "../../ui/state.js";
import { catalogKinds, catalogPacks, packSwitch, pageRiskFilters, pageRules, sheetRules, shownRules, sortPageRules, startGroups, suggestedReason, suggestedSplit, switchTurnsOn } from "../../ui/pack-model.js";
import { packSwitchWrites, settingWrites } from "../../ui/settings-writes.js";
import { isFromPack, publisherLabel, RISK_LABELS, worktreeLeftOut } from "../../ui/derive.js";
import { RULE_RISKS } from "../../src/engine/index.js";
import { packKind } from "../../ui/render/pack-panel.js";
import { changeOwner } from "../../ui/shell.js";

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

  it("is on when every rule is on, halfway when some are, and off when none is or there are none", () => {
    expect(packSwitch(rules(true, false, true))).toEqual({ on: 2, total: 3, state: "mixed" });
    expect(packSwitch(rules(true, true))).toEqual({ on: 2, total: 2, state: "true" });
    expect(packSwitch(rules(false, false))).toEqual({ on: 0, total: 2, state: "false" });
    expect(packSwitch([])).toEqual({ on: 0, total: 0, state: "false" });
  });

  it("halfway turns the rest on, then on turns everything off, then on again", () => {
    expect(packSwitch(rules(true, false, false)).state).toBe("mixed");
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
    expect(before).toMatchObject({ state: "mixed", on: 2 });

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
    expect(toast).toHaveBeenCalledWith(expect.stringMatching(/^Couldn’t save\. /u));
  });

  it("owns a change to the worktree's own rules by this worktree, and any other pack's by the repository", async () => {
    data.packs = [{ ...pack("worktree", "This worktree", "repository", ["wt:a"]), publisher: "worktree" }, { ...pack("typescript", "TypeScript", "language"), publisher: "rerail" }];
    data.checkout = { checkoutPath: "/work/a", folder: "", state: "started" };
    data.rules = toStoredRules([entry("wt:a", "", { source: { kind: "pack", id: "worktree" } }), entry("ts:a", "")]);
    data.attachedPacks = new Map([["worktree", [""]], ["typescript", [""]]]);
    const writes = settingWrites({ toast, invalidate: () => {}, owner: changeOwner });

    const switched = [writes.setEnabled(["wt:a"], false, "Off."), writes.setPackEnabled("worktree", false, "Off."), writes.setEnabled(["ts:a"], false, "Off.")];
    for (let i = 0; i < 4; i += 1) {
      await vi.waitFor(() => expect(puts()).toHaveLength(i + 1));
      puts()[i]!.answer(200, { settings: [] });
    }
    await Promise.all(switched);

    const bodies = puts().map(r => r.body as { owner: string; targets: Array<{ kind: string; id: string }> });
    expect(bodies.map(b => [b.owner, b.targets[0]!.id])).toEqual([
      ["checkout", "wt:a"], ["checkout", "worktree"], ["checkout", "wt:a"], ["repository", "ts:a"],
    ]);
    data.checkout = null;
  });
});

describe("the worktree's left-out rule files", () => {
  it("are the skipped entries in the checkout's own .rerail/rules, by file, whichever separator the daemon's system uses", () => {
    const skipped = [
      { path: "/home/me/.rerail/packs/node_modules/@acme/rerail-pack-sql", reason: "an installed pack" },
      { path: "/work/a/.rerail/rules/b.json", reason: "rule x is already in a.json" },
      { path: "/work/a/.rerail/rules", reason: "pack id worktree is taken by rerail-pack-worktree" },
      { path: "/work/ab/.rerail/rules/c.json", reason: "another checkout's" },
      { path: "C:\\work\\a\\.rerail\\rules\\d.json", reason: "on Windows" },
    ];

    expect(worktreeLeftOut(skipped, "/work/a")).toEqual([
      { file: "b.json", reason: "rule x is already in a.json" },
      { file: ".rerail/rules", reason: "pack id worktree is taken by rerail-pack-worktree" },
    ]);
    expect(worktreeLeftOut(skipped, "C:\\work\\a")).toEqual([{ file: "d.json", reason: "on Windows" }]);
    expect(worktreeLeftOut(skipped, undefined)).toEqual([]);
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

  it("offers the risks its rules have, in the risks' order with counts, and filters and searches by them", () => {
    const [typescript] = data.packs as { rules: { id: string; dimensions?: unknown }[] }[];
    typescript!.rules[0]!.dimensions = { risk: "data-safety", layer: "jobs", severity: "critical" };
    typescript!.rules[2]!.dimensions = { risk: "security", layer: "server", severity: "high", hazard: "auth.mutation-without-auth" };
    data.packs = [...data.packs];
    const all = pageRules("pack:typescript")!;
    const shown = (filter: string, query = "") => { Object.assign(ui2, { filter, query, sort: "az" }); return shownRules(all).map(r => r.rule.id); };

    expect(pageRiskFilters(all)).toEqual([["risk:security", "Security (1)"], ["risk:data-safety", "Data safety (1)"]]);
    expect(pageRiskFilters(pageRules("pack:next")!)).toEqual([]);
    expect(shown("risk:security")).toEqual(["ts:c"]);
    expect(shown("risk:data-safety")).toEqual(["ts:a"]);
    expect(shown("risk:performance")).toEqual([]);
    // A risk is found by its label and by its id as pack.json and the API write it.
    expect(shown("all", "data safety")).toEqual(["ts:a"]);
    expect(shown("all", "data-safety")).toEqual(["ts:a"]);
    expect(shown("all", "critical")).toEqual(["ts:a"]);
    expect(shown("all", "mutation-without-auth")).toEqual(["ts:c"]);
  });

  it("names every risk a pack may give", () => {
    expect(RISK_LABELS.map(([risk]) => risk)).toEqual([...RULE_RISKS]);
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

  it("puts the packs Rerail suggests for the repo first, once the recommendations name one", () => {
    const sorted = catalogPacks(packs, "all");
    const recommendations = [{ packId: "vue", reason: "package.json depends on vue" }, { packId: "python" }];

    expect(suggestedSplit(sorted, null)).toBeNull();
    expect(suggestedSplit(sorted, [])).toBeNull();
    const split = suggestedSplit(sorted, recommendations);
    expect(split?.suggested.map(p => p.id)).toEqual(["python", "vue"]);
    expect(split?.others.map(p => p.id)).toEqual(["blade", "next"]);
    // A kind with no suggested pack still splits: its packs are all others.
    expect(suggestedSplit(catalogPacks(packs, "language").filter(p => p.id === "blade"), recommendations)?.suggested).toEqual([]);
  });

  it("says why a pack is suggested, empty when the recommendation gives no reason", () => {
    const recommendations = [{ packId: "vue", reason: "package.json depends on vue" }, { packId: "python" }];

    expect(suggestedReason(recommendations, "vue")).toBe("package.json depends on vue");
    expect(suggestedReason(recommendations, "python")).toBe("");
    expect(suggestedReason(recommendations, "next")).toBeNull();
    expect(suggestedReason(null, "vue")).toBeNull();
  });

  it("groups a pack not added by how its rules start, keeping their order", () => {
    const rows = [{ id: "x:1", title: "Alpha", on: true }, { id: "x:2", title: "Beta", on: false }, { id: "x:3", title: "Gamma", on: true }];

    expect(startGroups(rows)).toEqual({ on: [rows[0], rows[2]], off: [rows[1]] });
  });

  it("filters a pack sheet's rules by title or id, A to Z", () => {
    const p = { ...pack("x", "X", "language"), rules: [{ id: "x:2", title: "Beta" }, { id: "x:1", title: "Alpha" }, { id: "x:3", title: "Gamma" }] };

    expect(sheetRules(p, "").map(r => r.id)).toEqual(["x:1", "x:2", "x:3"]);
    expect(sheetRules(p, " ETA ").map(r => r.id)).toEqual(["x:2"]);
    expect(sheetRules(p, "x:3").map(r => r.id)).toEqual(["x:3"]);
  });

  it("names a third party's pack by its publisher, always marked as not reviewed, and Rerail's own as before", () => {
    const sql = { ...pack("sql", "SQL", "language"), publisher: "rerail" };
    const scoped = { ...pack("acme-sql", "Acme SQL", "language"), publisher: "@acme" };
    const unscoped = { ...pack("lint", "Lint", "repository"), publisher: "rerail-pack-lint" };
    // A scope that reads like Rerail's is still a third party's.
    const lookalike = { ...pack("official", "Official", "language"), publisher: "@rerail" };

    expect([sql, scoped, unscoped, lookalike].map(publisherLabel))
      .toEqual(["By Rerail", "By @acme · not reviewed", "By rerail-pack-lint · not reviewed", "By @rerail · not reviewed"]);
    expect([sql, scoped, unscoped].map(packKind)).toEqual(["Language", "Language · By @acme · not reviewed", "Repository · By rerail-pack-lint · not reviewed"]);
  });

  it("says a pack came from a folder or tarball, never By Rerail, whatever its package.json names", () => {
    const local = { ...pack("sql", "SQL", "language"), publisher: "rerail", installedFrom: "folder" };
    const acme = { ...pack("acme-sql", "Acme SQL", "language"), publisher: "@acme", installedFrom: "tarball" };

    expect([local, acme].map(publisherLabel)).toEqual(["From a folder · not reviewed", "By @acme · From a tarball · not reviewed"]);
    expect(packKind(local)).toBe("Language · From a folder · not reviewed");
  });

  it("takes every rule a pack brings as from a pack, and only the library's as created by you", () => {
    const rules = toStoredRules([entry("p", ""), entry("r", "", { source: { kind: "rerail_rule", id: "r", packId: null } }), entry("u", "", { source: { kind: "library_rule", id: "u", packId: null } })]);

    expect(["p", "r", "u"].map(id => isFromPack(rules.get(id)!))).toEqual([true, true, false]);
  });
});
