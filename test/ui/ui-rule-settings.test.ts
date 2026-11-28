import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { changeScope, toStoredRules, type StoredRule } from "../../ui/poll.js";
import { data } from "../../ui/state.js";
import { inPack, packSuppressions } from "../../ui/derive.js";
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

describe("a rule pack's suppression comments on the page", () => {
  it("sums up an added pack's rules for its sheet", () => {
    const honouring = [...toStoredRules([entry("eslint:no-var", ""), entry("eslint:radix", "")]).values()];
    const one = [...toStoredRules([entry("eslint:no-var", ""), entry("eslint:radix", "", { suppressions: "ignore" })]).values()];
    const all = [...toStoredRules([entry("eslint:no-var", "", { suppressions: "ignore" }), entry("eslint:radix", "", { suppressions: "ignore" })]).values()];

    expect(packSuppressions(honouring)).toBe("honoured by its rules");
    expect(packSuppressions(one)).toBe("ignored by 1 of its 2 rules");
    expect(packSuppressions(all)).toBe("ignored by its rules");
  });

  it("counts a rule that ignores them in any folder it is added at", () => {
    const rules = [...toStoredRules([
      entry("eslint:no-var", "", { suppressions: "honour", suppressionsSource: inRepository("", "rule") }),
      entry("eslint:no-var", "web", { suppressions: "ignore", suppressionsSource: inRepository("web", "pack") }),
    ]).values()];
    expect(packSuppressions(rules)).toBe("ignored by its rules");
  });

  // Self-review round 8: a rule added on its own in one folder and by the pack in another is still the pack's rule,
  // whichever folder the page lists first.
  it("finds the pack's rules whichever folder the pack added them at", () => {
    const [rule] = [...toStoredRules([
      entry("eslint:no-var", "a", { source: { kind: "rerail_rule", id: "eslint:no-var" }, suppressions: "honour", suppressionsSource: { by: "pack-start" } }),
      entry("eslint:no-var", "b", { suppressions: "ignore", suppressionsSource: inRepository("b", "pack") }),
    ]).values()];
    expect(inPack(rule!, "eslint")).toBe(true);
    expect(inPack(rule!, "other")).toBe(false);
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
    vi.stubGlobal("fetch", (_input: URL, init: { method?: string; body?: string; signal?: AbortSignal } = {}) => {
      const { promise, resolve, reject } = Promise.withResolvers<Response>();
      // As fetch does: a request whose signal aborts rejects with the signal's reason.
      init.signal?.addEventListener("abort", () => reject(init.signal!.reason));
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
    vi.useRealTimers();
    vi.unstubAllGlobals();
    data.rules = new Map();
    data.attachedPacks = new Map();
  });

  it("switches a rule at every folder it is added at, owned as the page says", async () => {
    owner = "checkout";
    const saving = page().setEnabled(["eslint:no-var"], false, "Off.");

    expect(stored("eslint:no-var").enabled).toBe(false);
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toEqual({
      repository: "A", checkout: "/work/a", folder: "", owner: "checkout", targets: [{ kind: "rule", id: "eslint:no-var" }], on: false,
    });
    writes()[0]!.answer(200, {});
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    expect(writes()[1]!.body).toMatchObject({ folder: "web", owner: "checkout", targets: [{ kind: "rule", id: "eslint:no-var" }], on: false });
    writes()[1]!.answer(200, {});
    await saving;

    expect(toast).toHaveBeenCalledWith("Off.");
  });

  it("puts the rule back as it was when the write fails", async () => {
    const saving = page().setEnabled(["eslint:no-var"], false, "Off.");
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    writes()[0]!.answer(503, { error: "daemon restarting" });
    await saving;

    expect(stored("eslint:no-var").enabled).toBe(true);
    expect(toast).toHaveBeenCalledWith(expect.stringMatching(/^Couldn’t save\. PUT \/api\/repository\/settings answered 503/u));
  });

  // Review finding F6: a write the daemon never answers fails after a while, so later clicks for the repository go out.
  it("gives up on a write the daemon never answers and sends the next one", async () => {
    vi.useFakeTimers();
    const hung = page().setPackEnabled("eslint", false, "Off.");
    const next = page().setEnabled(["eslint:radix"], true, "On.");
    await vi.advanceTimersByTimeAsync(0);
    expect(writes()).toHaveLength(1);
    await vi.advanceTimersByTimeAsync(29_999);
    expect(writes()).toHaveLength(1);

    await vi.advanceTimersByTimeAsync(1);
    await hung;
    expect(toast).toHaveBeenCalledWith("Couldn’t save. The daemon did not answer within 30 s.");
    expect(stored("eslint:no-var").enabled).toBe(true);
    await vi.advanceTimersByTimeAsync(0);
    expect(writes()).toHaveLength(2);
    expect(writes()[1]!.body).toMatchObject({ targets: [{ kind: "rule", id: "eslint:radix" }], on: true });
    writes()[1]!.answer(200, {});
    await next;
    expect(toast).toHaveBeenCalledWith("On.");
  });

  // Self-review rounds 10 and 11: writes go out in click order; after a failure the page keeps what a saved write
  // decides and puts back a write that saved nothing without undoing later clicks.
  it("sends writes in click order and, after a failure, keeps what was saved", async () => {
    const partial = page().setPackEnabled("eslint", false, "Off.");
    const enabling = page().setEnabled(["eslint:radix"], true, "On.");
    const disabling = page().setEnabled(["eslint:eqeqeq"], false, "Off.");
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    expect(writes()[0]!.body).toMatchObject({ folder: "", targets: [{ kind: "pack", id: "eslint" }], on: false });
    writes()[0]!.answer(200, {});
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    writes()[1]!.answer(503, { error: "daemon restarting" });
    await partial;
    expect(stored("eslint:no-var").enabled).toBe(false);

    // The radix write was clicked before the eqeqeq one: it goes out first, and the other waits for its answer.
    await vi.waitFor(() => expect(writes()).toHaveLength(3));
    expect(writes()[2]!.body).toMatchObject({ targets: [{ kind: "rule", id: "eslint:radix" }], on: true });
    writes()[2]!.answer(503, { error: "daemon restarting" });
    await enabling;
    await vi.waitFor(() => expect(writes()).toHaveLength(4));
    expect(writes()[3]!.body).toMatchObject({ targets: [{ kind: "rule", id: "eslint:eqeqeq" }], on: false });
    writes()[3]!.answer(200, {});
    await disabling;
    // Radix is back off, as the pack switch left it when it was clicked on.
    expect(stored("eslint:radix").enabled).toBe(false);
    expect(stored("eslint:eqeqeq").enabled).toBe(false);
  });

  // Self-review rounds 12 and 13: a write another repository's page left unanswered does not hold this one's back, and
  // one for the same repository still goes out first when the page comes back to it.
  it("keeps click order per repository across switches, and never makes another repository wait", async () => {
    void page().setEnabled(["eslint:no-var"], false, "Off.");
    await vi.waitFor(() => expect(writes()).toHaveLength(1));
    changeScope({ repository: "B", checkout: "/work/b" });
    data.rules = toStoredRules([entry("eslint:radix", "")]);
    void page().setEnabled(["eslint:radix"], false, "Off.");
    await vi.waitFor(() => expect(writes()).toHaveLength(2));
    expect(writes()[1]!.body).toMatchObject({ repository: "B", targets: [{ kind: "rule", id: "eslint:radix" }] });
    writes()[1]!.answer(200, {});

    changeScope({ repository: "A", checkout: "/work/a" });
    data.rules = toStoredRules([entry("eslint:no-var", "")]);
    const again = page().setEnabled(["eslint:no-var"], false, "Off.");
    // Nothing the page could do on its own would send it: let every pending reaction run, then look.
    for (let turn = 0; turn < 20; turn += 1) await Promise.resolve();
    expect(writes()).toHaveLength(2);
    writes()[0]!.answer(200, {});
    await vi.waitFor(() => expect(writes()).toHaveLength(3));
    expect(writes()[2]!.body).toMatchObject({ repository: "A", on: false });
    writes()[2]!.answer(200, {});
    await again;
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
