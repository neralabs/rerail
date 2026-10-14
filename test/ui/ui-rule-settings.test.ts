import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { changeScope, toStoredRules, type StoredRule } from "../../ui/poll.js";
import { data } from "../../ui/state.js";
import { inPack, nextSuppressions, optionsText, packSuppressions, suppressionsText } from "../../ui/derive.js";
import { settingWrites } from "../../ui/settings-writes.js";

type Request = { method: string; body: unknown; answer: (status: number, body?: unknown) => void };

const packStart = { by: "pack-start", packId: "eslint" };
const inRepository = (folder: string, target: "rule" | "pack") => ({ by: "repository", folder, target });
const inWorktree = (folder: string, target: "rule" | "pack") => ({ by: "checkout", checkoutPath: "/work/a", folder, target });

/** One entry of `GET /api/repository/rules`: a rule present through the eslint pack at `folder`. */
const entry = (id: string, folder: string, fields: Record<string, unknown> = {}) => ({
  kind: "file",
  rule: { id, mode: "steer", clauses: [] },
  folder,
  on: true,
  mode: "steer",
  source: { kind: "pack", id: "eslint", packId: "eslint" },
  onSource: packStart,
  modeSource: packStart,
  suppressions: "honour",
  suppressionsSource: packStart,
  config: null,
  configSource: {},
  revision: 1,
  ...fields,
});

describe("a rule's suppression comments and options on the page", () => {
  it("shows them honoured or ignored with what decided it", () => {
    const rules = toStoredRules([
      entry("eslint:no-var", ""),
      entry("eslint:eqeqeq", "", { suppressions: "ignore", suppressionsSource: inWorktree("", "pack") }),
      entry("eslint:radix", "", { suppressions: "ignore", suppressionsSource: inRepository("web", "rule") }),
    ]);

    expect(suppressionsText(rules.get("eslint:no-var")!, "/work/a")).toBe("Honoured · Rule pack eslint default");
    expect(suppressionsText(rules.get("eslint:eqeqeq")!, "/work/a")).toBe("Ignored · Set in this worktree for its rule pack");
    expect(suppressionsText(rules.get("eslint:radix")!, "/work/a")).toBe("Ignored · Set for the repository at web");
  });

  it("lists each folder's own when the folders the rule is added at disagree, and switches to ignore", () => {
    const rule = toStoredRules([
      entry("eslint:no-var", ""),
      entry("eslint:no-var", "web", { suppressions: "ignore", suppressionsSource: inRepository("web", "rule") }),
    ]).get("eslint:no-var")!;

    expect(rule.folderStates.map(state => state.suppressions)).toEqual(["honour", "ignore"]);
    expect(rule.mixed).toBe(false);
    expect(suppressionsText(rule)).toBe("Honoured in Whole repository, Ignored in web");
    expect(nextSuppressions(rule)).toBe("ignore");
  });

  it("switches to honour once every folder ignores them", () => {
    const rule = toStoredRules([entry("eslint:no-var", "", { suppressions: "ignore", suppressionsSource: inRepository("", "rule") })]).get("eslint:no-var")!;

    expect(nextSuppressions(rule)).toBe("honour");
  });

  it("lists configured options with what decided each, and none when the rule has none", () => {
    const rules = toStoredRules([
      entry("eslint:max-depth", "", { config: { max: 3, style: "always" }, configSource: { max: inRepository("", "rule") } }),
      entry("eslint:no-var", ""),
    ]);

    expect(optionsText(rules.get("eslint:max-depth")!)).toBe("max: 3 · Set for the repository, style: \"always\"");
    expect(optionsText(rules.get("eslint:no-var")!)).toBeNull();
  });

  it("sums up an added pack's rules for its panel", () => {
    const honouring = [...toStoredRules([entry("eslint:no-var", ""), entry("eslint:radix", "")]).values()];
    const one = [...toStoredRules([entry("eslint:no-var", ""), entry("eslint:radix", "", { suppressions: "ignore" })]).values()];
    const all = [...toStoredRules([entry("eslint:no-var", "", { suppressions: "ignore" }), entry("eslint:radix", "", { suppressions: "ignore" })]).values()];

    expect(packSuppressions(honouring, "eslint", "repository")).toEqual({ text: "honoured by its rules", next: "ignore" });
    expect(packSuppressions(one, "eslint", "repository")).toEqual({ text: "ignored by 1 of its 2 rules", next: "ignore" });
    expect(packSuppressions(all, "eslint", "repository")).toEqual({ text: "ignored by its rules", next: "honour" });
  });

  // Self-review round 4: the switch writes only the pack's setting, so a rule with its own setting does not hold it on
  // ignore forever.
  it("offers to honour them again once every rule the pack's setting decides ignores them", () => {
    const rules = [...toStoredRules([
      entry("eslint:no-var", "", { suppressions: "ignore", suppressionsSource: inRepository("", "pack") }),
      entry("eslint:radix", "", { suppressions: "honour", suppressionsSource: inRepository("", "rule") }),
    ]).values()];
    expect(packSuppressions(rules, "eslint", "repository")).toEqual({ text: "ignored by 1 of its 2 rules", next: "honour" });
    // A worktree's own setting decides a rule for the repository's switch, not for the worktree's.
    const worktree = [...toStoredRules([entry("eslint:no-var", "", { suppressions: "honour", suppressionsSource: inWorktree("", "pack") })]).values()];
    expect(packSuppressions(worktree, "eslint", "repository", "/work/a").next).toBe("ignore");
    expect(packSuppressions(worktree, "eslint", "checkout", "/work/a").next).toBe("ignore");
  });

  // Self-review round 5: the pack's setting decides each folder on its own; a rule honouring them in one folder by its
  // own setting kept the switch on ignore, since only the rule's first folder's decider was looked at.
  it("offers to honour them when only a folder the pack's setting does not decide honours them", () => {
    const rules = [...toStoredRules([
      entry("eslint:no-var", "", { suppressions: "ignore", suppressionsSource: inRepository("", "pack") }),
      entry("eslint:no-var", "web", { suppressions: "honour", suppressionsSource: inRepository("web", "rule") }),
      entry("eslint:radix", "", { suppressions: "ignore", suppressionsSource: inRepository("", "pack") }),
      entry("eslint:radix", "web", { suppressions: "ignore", suppressionsSource: inRepository("web", "pack") }),
    ]).values()];
    expect(packSuppressions(rules, "eslint", "repository")).toEqual({ text: "ignored by its rules", next: "honour" });
  });

  // Self-review round 6: the pack's setting decides a folder by the resolver's precedence, so a pack setting at a
  // deeper folder or a nearer owner wins over a rule's own setting; only the same owner and folder keeps the rule's.
  it("counts a folder where the pack's setting would win over a rule's own setting by owner or depth", () => {
    const rootRuleIgnores = [...toStoredRules([
      entry("eslint:no-var", "web", { suppressions: "ignore", suppressionsSource: inRepository("", "rule") }),
      entry("eslint:radix", "web", { suppressions: "ignore", suppressionsSource: inRepository("", "rule") }),
    ]).values()];
    // The pack attached at `web`: its setting there is deeper than the rules' at the repository root.
    expect(packSuppressions(rootRuleIgnores, "eslint", "repository", "/work/a")).toEqual({ text: "ignored by its rules", next: "honour" });
    // This worktree's pack setting outranks the repository's rule setting.
    expect(packSuppressions(rootRuleIgnores, "eslint", "checkout", "/work/a").next).toBe("honour");
    // At the same owner and folder the rule's own setting wins.
    const sameFolder = [...toStoredRules([
      entry("eslint:no-var", "web", { suppressions: "ignore", suppressionsSource: inRepository("web", "rule") }),
    ]).values()];
    expect(packSuppressions(sameFolder, "eslint", "repository", "/work/a").next).toBe("ignore");
    // A worktree containing this one outranks the repository but not this worktree.
    const parent = [...toStoredRules([entry("eslint:no-var", "", {
      suppressions: "ignore", suppressionsSource: { by: "checkout", checkoutPath: "/work", folder: "", target: "rule" },
    })]).values()];
    expect(packSuppressions(parent, "eslint", "repository", "/work/a").next).toBe("ignore");
    expect(packSuppressions(parent, "eslint", "checkout", "/work/a").next).toBe("honour");
  });

  // Self-review round 7: a folder where the rule is added on its own, not by the pack, is not the pack's to change.
  it("counts only the folders where the pack added the rule", () => {
    const rules = [...toStoredRules([
      entry("eslint:no-var", "a", { suppressions: "ignore", suppressionsSource: inRepository("a", "pack") }),
      entry("eslint:no-var", "b", { source: { kind: "rerail_rule", id: "eslint:no-var" }, suppressions: "honour", suppressionsSource: { by: "pack-start" } }),
    ]).values()];
    expect(packSuppressions(rules, "eslint", "repository")).toEqual({ text: "ignored by its rules", next: "honour" });
    expect(packSuppressions(rules, "other", "repository").next).toBe("ignore");
  });

  // Self-review round 8: a rule added on its own in one folder and by the pack in another is still the pack's rule,
  // whichever folder the page lists first.
  it("finds the pack's rules whichever folder the pack added them at", () => {
    const [rule] = [...toStoredRules([
      entry("eslint:no-var", "a", { source: { kind: "rerail_rule", id: "eslint:no-var" }, suppressions: "honour", suppressionsSource: { by: "pack-start" } }),
      entry("eslint:no-var", "b", { suppressions: "ignore", suppressionsSource: inRepository("b", "pack") }),
    ]).values()];
    expect(inPack(rule!, "eslint")).toBe(true);
    expect(packSuppressions([rule!], "eslint", "repository")).toEqual({ text: "ignored by its rules", next: "honour" });
  });
});

describe("the page's rule setting writes", () => {
  let requests: Request[] = [];
  const writes = () => requests.filter(r => r.method === "PUT");
  const toast = vi.fn();
  let owner: "repository" | "checkout" = "repository";
  const page = () => settingWrites({ toast, invalidate: () => {}, owner: () => owner });
  const stored = (id: string) => (data.rules as Map<string, StoredRule>).get(id)!;

  beforeEach(() => {
    requests = [];
    toast.mockReset();
    owner = "repository";
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
      entry("eslint:no-var", ""),
      entry("eslint:no-var", "web"),
      entry("eslint:radix", "", { suppressions: "honour", suppressionsSource: inRepository("", "rule") }),
      entry("eslint:eqeqeq", "", { suppressions: "honour", suppressionsSource: inWorktree("", "pack") }),
    ]);
    data.attachedPacks = new Map([["eslint", ["", "web"]]]);
  });

  afterEach(() => {
    vi.unstubAllGlobals();
    data.rules = new Map();
    data.attachedPacks = new Map();
  });

  it("ignores suppression comments for a rule at every folder it is added at, owned as the page says", async () => {
    owner = "checkout";
    const saving = page().setSuppressions(["eslint:no-var"], "ignore", "Ignored.");

    expect(stored("eslint:no-var").folderStates.map(state => state.suppressions)).toEqual(["ignore", "ignore"]);
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toEqual({
      repository: "A", checkout: "/work/a", folder: "", owner: "checkout", targets: [{ kind: "rule", id: "eslint:no-var" }], suppressions: "ignore",
    });
    writes()[0]!.answer(200, {});
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    expect(writes()[1]!.body).toMatchObject({ folder: "web", owner: "checkout", targets: [{ kind: "rule", id: "eslint:no-var" }], suppressions: "ignore" });
    writes()[1]!.answer(200, {});
    await saving;

    expect(toast).toHaveBeenCalledWith("Ignored.");
    expect(stored("eslint:no-var").suppressions).toBe("ignore");
  });

  // Self-review round 8: this worktree's own setting outranks the repository's, so a repository change does not show
  // in a folder the worktree decides.
  it("shows a repository change only in the folders this worktree's setting does not decide", async () => {
    data.rules = toStoredRules([
      entry("eslint:no-var", "", { suppressions: "honour", suppressionsSource: inWorktree("", "rule") }),
      entry("eslint:no-var", "web", { suppressions: "honour", suppressionsSource: inRepository("web", "rule") }),
    ]);
    const saving = page().setSuppressions(["eslint:no-var"], "ignore", "Ignored.");
    expect(stored("eslint:no-var").folderStates.map(state => state.suppressions)).toEqual(["honour", "ignore"]);
    for (let index = 0; index < 2; index += 1) {
      await vi.waitFor(() => expect(writes()).toHaveLength(index + 1));
      writes()[index]!.answer(200, {});
    }
    await saving;
  });

  // Self-review round 9: a rule's own setting, just written, outranks its pack's at the same owner and folder, so a pack
  // change made before the page refreshes does not show on it.
  it("weighs a change not yet refreshed when showing the next one", async () => {
    let done = 0;
    const answerPending = () => vi.waitFor(() => {
      expect(writes().length).toBeGreaterThan(done);
      for (const write of writes().slice(done)) write.answer(200, {});
      done = writes().length;
    });
    const rule = page().setSuppressions(["eslint:no-var"], "ignore", "Ignored.");
    expect(stored("eslint:no-var").folderStates.map(state => state.suppressionsSource)).toEqual([inRepository("", "rule"), inRepository("web", "rule")]);
    const pack = page().setPackSuppressions("eslint", "honour", "Honoured for ESLint.");
    expect(stored("eslint:no-var").folderStates.map(state => state.suppressions)).toEqual(["ignore", "ignore"]);
    while (done < 4) await answerPending();
    await Promise.all([rule, pack]);
    expect(stored("eslint:no-var").suppressions).toBe("ignore");
  });

  it("puts the rule back as it was when the write fails", async () => {
    const saving = page().setSuppressions(["eslint:no-var"], "ignore", "Ignored.");
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.answer(503, { error: "daemon restarting" });
    await saving;

    expect(stored("eslint:no-var").suppressions).toBe("honour");
    expect(stored("eslint:no-var").folderStates.map(state => state.suppressions)).toEqual(["honour", "honour"]);
    expect(toast).toHaveBeenCalledWith(expect.stringMatching(/^Could not save: PUT \/api\/repository\/settings answered 503/u));
  });

  // Self-review rounds 10 and 11: writes go out in click order; after a failure the page keeps what a saved folder
  // decides (the root's setting reaches web too) and puts back a write that saved nothing without undoing later clicks.
  it("sends writes in click order and, after a failure, keeps what was saved", async () => {
    const partial = page().setSuppressions(["eslint:no-var"], "ignore", "Ignored.");
    const enabling = page().setEnabled(["eslint:radix"], false, "Off.");
    const ignoring = page().setSuppressions(["eslint:radix"], "ignore", "Ignored.");
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.answer(200, {});
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    writes()[1]!.answer(503, { error: "daemon restarting" });
    await partial;
    expect(stored("eslint:no-var").folderStates.map(state => [state.suppressions, state.suppressionsSource])).toEqual([
      ["ignore", inRepository("", "rule")], ["ignore", inRepository("web", "rule")],
    ]);

    // The on/off write was clicked before the suppressions one: it goes out first, and the other waits for its answer.
    await vi.waitFor(() => expect(writes()).toHaveLength(3));
    expect(writes()[2]!.body).toMatchObject({ on: false });
    writes()[2]!.answer(503, { error: "daemon restarting" });
    await enabling;
    await vi.waitFor(() => expect(writes()).toHaveLength(4));
    expect(writes()[3]!.body).toMatchObject({ suppressions: "ignore" });
    writes()[3]!.answer(200, {});
    await ignoring;
    expect(stored("eslint:radix")).toMatchObject({ enabled: true, suppressions: "ignore", suppressionsSource: inRepository("", "rule") });
  });

  // Self-review rounds 12 and 13: a write another repository's page left unanswered does not hold this one's back, and
  // one for the same repository still goes out first when the page comes back to it.
  it("keeps click order per repository across switches, and never makes another repository wait", async () => {
    void page().setSuppressions(["eslint:no-var"], "honour", "Honoured.");
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    changeScope({ repository: "B", checkout: "/work/b" });
    data.rules = toStoredRules([entry("eslint:radix", "")]);
    void page().setSuppressions(["eslint:radix"], "ignore", "Ignored.");
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    expect(writes()[1]!.body).toMatchObject({ repository: "B", targets: [{ kind: "rule", id: "eslint:radix" }] });
    writes()[1]!.answer(200, {});

    changeScope({ repository: "A", checkout: "/work/a" });
    data.rules = toStoredRules([entry("eslint:no-var", "")]);
    const again = page().setSuppressions(["eslint:no-var"], "ignore", "Ignored.");
    // Nothing the page could do on its own would send it: let every pending reaction run, then look.
    for (let turn = 0; turn < 20; turn += 1) await Promise.resolve();
    expect(writes()).toHaveLength(2);
    writes()[0]!.answer(200, {});
    await vi.waitFor(() => expect(writes()).toHaveLength(3));
    expect(writes()[2]!.body).toMatchObject({ repository: "A", suppressions: "ignore" });
    writes()[2]!.answer(200, {});
    await again;
  });

  it("sets a pack at every folder it is added at, shown on its rules that follow the pack", async () => {
    const saving = page().setPackSuppressions("eslint", "ignore", "Ignored for ESLint.");

    // A rule with its own setting keeps it; one this worktree decides keeps it when the repository's changes.
    expect(stored("eslint:no-var").suppressions).toBe("ignore");
    expect(stored("eslint:radix").suppressions).toBe("honour");
    expect(stored("eslint:eqeqeq").suppressions).toBe("honour");
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toEqual({
      repository: "A", checkout: "/work/a", folder: "", owner: "repository", targets: [{ kind: "pack", id: "eslint" }], suppressions: "ignore",
    });
    writes()[0]!.answer(200, {});
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    expect(writes()[1]!.body).toMatchObject({ folder: "web", targets: [{ kind: "pack", id: "eslint" }], suppressions: "ignore" });
    writes()[1]!.answer(200, {});
    await saving;

    expect(toast).toHaveBeenCalledWith("Ignored for ESLint.");
  });

  // Self-review round 5: a folder where the rule has its own setting keeps it, while the pack's other folders change.
  it("shows a pack change only in the folders the pack's setting decides", async () => {
    data.rules = toStoredRules([
      entry("eslint:no-var", "", { suppressions: "honour", suppressionsSource: inRepository("", "pack") }),
      entry("eslint:no-var", "web", { suppressions: "honour", suppressionsSource: inRepository("web", "rule") }),
    ]);
    const saving = page().setPackSuppressions("eslint", "ignore", "Ignored for ESLint.");

    expect(stored("eslint:no-var").folderStates.map(state => state.suppressions)).toEqual(["ignore", "honour"]);
    expect(stored("eslint:no-var").suppressions).toBe("ignore");
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.answer(200, {});
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    writes()[1]!.answer(200, {});
    await saving;
  });

  // Self-review round 6: this worktree's pack setting outranks the repository's setting for the rule.
  it("shows a worktree's pack change on a rule the repository's rule setting decides", async () => {
    owner = "checkout";
    data.checkout = { checkoutPath: "/work/a", folder: "", state: "started" };
    const saving = page().setPackSuppressions("eslint", "ignore", "Ignored for ESLint.");

    expect(stored("eslint:radix").suppressions).toBe("ignore");
    expect(stored("eslint:eqeqeq").suppressions).toBe("ignore");
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.answer(200, {});
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    writes()[1]!.answer(200, {});
    await saving;
  });

  it("switches rules on or off with one write per folder, each naming the rules added there", async () => {
    const saving = page().setEnabled(["eslint:no-var", "eslint:radix"], false, "Off.");

    expect(stored("eslint:no-var").enabled).toBe(false);
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toMatchObject({ folder: "", owner: "repository", targets: [{ kind: "rule", id: "eslint:no-var" }, { kind: "rule", id: "eslint:radix" }], on: false });
    writes()[0]!.answer(200, {});
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    expect(writes()[1]!.body).toMatchObject({ folder: "web", targets: [{ kind: "rule", id: "eslint:no-var" }], on: false });
    writes()[1]!.answer(500, { error: "disk full" });
    await saving;

    // The repository root was saved: both rules are off there now, so neither is on in every folder.
    expect(stored("eslint:no-var").enabled).toBe(false);
    expect(stored("eslint:radix").enabled).toBe(false);
  });
});
