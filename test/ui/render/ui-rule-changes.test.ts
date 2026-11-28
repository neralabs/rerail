import { describe, expect, it } from "vitest";
import { changeCounts, changedParts, changedSince, editedCopyIn, lastEditedAt, setToText } from "../../../ui/render/rule-changes.js";
import { toStoredRules } from "../../../ui/poll.js";

const DAY = 24 * 60 * 60 * 1000;
const SEEN = 10 * DAY;

type Extra = { addedAt?: number; updatedAt?: number | null; copiedFromRuleId?: string | null; enabled?: boolean; mode?: string; origin?: string; waits?: boolean };
const rule = (id: string, extra: Extra = {}) => ({
  kind: "code", origin: extra.origin ?? "user", enabled: extra.enabled ?? true,
  rule: { id, title: id, mode: extra.mode ?? "steer", description: "d", clauses: [{ id: "main", violation: { message: `${id} says` } }] },
  addedAt: extra.addedAt ?? SEEN - DAY, updatedAt: extra.updatedAt ?? null, copiedFromRuleId: extra.copiedFromRuleId ?? null,
  // A rule Rerail did not publish carries its approval; one that waits runs in Log until a person approves it.
  approval: { approved: extra.waits !== true },
});
const ids = (rules: ReadonlyArray<{ rule: { id: string } }>) => rules.map(stored => stored.rule.id);

describe("the rules created by you that run as set and changed since the notice was last dismissed", () => {
  it("counts a rule added since then as added, and leaves out one added before", () => {
    const changes = changedSince([rule("new", { addedAt: SEEN + 1 }), rule("old")], SEEN);
    expect(ids(changes.added)).toEqual(["new"]);
    expect(changes.edited).toEqual([]);
  });

  it("counts an edited copy of another rule as edited, not added", () => {
    const changes = changedSince([rule("no-any-copy", { addedAt: SEEN + 1, copiedFromRuleId: "no-any" })], SEEN);
    expect(changes.added).toEqual([]);
    expect(ids(changes.edited)).toEqual(["no-any-copy"]);
  });

  it("counts a rule of yours edited in place since then as edited", () => {
    const changes = changedSince([rule("mine", { updatedAt: SEEN + 5 }), rule("untouched", { updatedAt: SEEN - DAY })], SEEN);
    expect(ids(changes.edited)).toEqual(["mine"]);
  });

  it("counts a rule both added and edited since then once, as added", () => {
    const changes = changedSince([rule("new", { addedAt: SEEN + 1, updatedAt: SEEN + 9 })], SEEN);
    expect(ids(changes.added)).toEqual(["new"]);
    expect(changes.edited).toEqual([]);
  });

  it("leaves out a rule that waits for approval, which the approval banner announces", () => {
    const rules = [rule("waits", { addedAt: SEEN + 1, waits: true }), rule("no-any-copy", { addedAt: SEEN + 1, copiedFromRuleId: "no-any", waits: true })];
    expect(changedSince(rules, SEEN)).toEqual({ added: [], edited: [] });
  });

  it("leaves out a pack's rules, which the dashboard's own switches change", () => {
    const changes = changedSince([rule("eslint-no-debugger", { origin: "pack", addedAt: SEEN + 1 })], SEEN);
    expect(changes).toEqual({ added: [], edited: [] });
  });

  it("says how many of each", () => {
    expect(changeCounts({ added: [rule("a"), rule("b")], edited: [rule("c")] })).toBe("2 added and 1 edited");
    expect(changeCounts({ added: [rule("a")], edited: [] })).toBe("1 added");
    expect(changeCounts({ added: [], edited: [rule("c")] })).toBe("1 edited");
  });
});

describe("how the changed rules are set", () => {
  it("is one clause when they are all set alike", () => {
    expect(setToText([rule("a"), rule("b")])).toBe("set to Steer");
    expect(setToText([rule("a", { mode: "log" })])).toBe("set to Log only");
  });

  it("counts each setting when they differ, a rule switched off included", () => {
    expect(setToText([rule("a"), rule("b"), rule("c", { mode: "log" })])).toBe("2 set to Steer, 1 set to Log only");
    expect(setToText([rule("a"), rule("b", { enabled: false })])).toBe("1 set to Steer, 1 off");
  });
});

describe("a rule's last edit", () => {
  it("is when a rule created by you was edited after it was added", () => {
    expect(lastEditedAt(rule("mine", { addedAt: 100, updatedAt: 250 }))).toBe(250);
  });

  it("is nothing for a rule never edited, and for a pack's rule", () => {
    expect(lastEditedAt(rule("mine", { addedAt: 100, updatedAt: 100 }))).toBeNull();
    expect(lastEditedAt(rule("mine", { addedAt: 100, updatedAt: null }))).toBeNull();
    expect(lastEditedAt(rule("eslint-no-debugger", { origin: "pack", addedAt: 100, updatedAt: 250 }))).toBeNull();
  });
});

describe("an edited copy", () => {
  it("is found for its original, the newest when there are several", () => {
    const original = rule("no-any", { origin: "pack" });
    const rules = [original, rule("no-any-copy", { copiedFromRuleId: "no-any", addedAt: 5 }), rule("no-any-copy-2", { copiedFromRuleId: "no-any", addedAt: 9 })];
    expect(editedCopyIn(rules, original)?.rule.id).toBe("no-any-copy-2");
    expect(editedCopyIn(rules, rule("other", { origin: "pack" }))).toBeNull();
  });

  it("does not stand in for its original while it waits for approval: the original keeps running until then", () => {
    const original = rule("no-any", { origin: "pack" });
    expect(editedCopyIn([original, rule("no-any-copy", { copiedFromRuleId: "no-any", waits: true })], original)).toBeNull();
  });

  it("names the parts of the details an edit changed, in the details' order", () => {
    const was = rule("no-debugger", { origin: "pack" });
    const now = rule("no-debugger-copy", { copiedFromRuleId: "no-debugger" });
    now.rule.clauses = [{ id: "main", violation: { message: "Remove this debugger statement." } }];
    Object.assign(now.rule, { scope: { include: ["**/*.ts"], exclude: ["scripts/**"] } });
    expect(changedParts(was, now)).toEqual(["What the agent sees", "Where it applies"]);
    expect(changedParts(was, { ...was })).toEqual([]);
    expect(changedParts(was, rule("no-debugger-copy", { mode: "log" }))).toContain("On a violation");
    // A copy that waits is held on Log whatever it is set to: that is the approval's doing, not an edit.
    expect(changedParts(was, { ...rule("no-debugger-copy", { mode: "log" }), modeSource: { by: "awaiting-approval" } })).not.toContain("On a violation");
  });

  it("reads which rule it was edited from off the rules answer", () => {
    // Two entries of `GET /api/repository/rules`, as the daemon sends them: an edited copy names its original, any other rule null.
    const entry = (id: string, copiedFromRuleId: string | null) => ({
      kind: "code", rule: { id, mode: "steer", clauses: [] }, folder: "", on: true, mode: "steer",
      source: { kind: "library_rule", id, packId: null }, onSource: { by: "rule-default" }, modeSource: { by: "rule-default" },
      suppressions: "honour", suppressionsSource: { by: "rule-default" }, config: null, configSource: {}, configError: null,
      revision: 2, payloadHash: "sha256:0", addedAt: 1, updatedAt: 1, copiedFromRuleId,
    });
    const stored = toStoredRules([entry("no-console-copy", "no-console"), entry("agents-rule", null)]);

    expect(stored.get("no-console-copy")!.copiedFromRuleId).toBe("no-console");
    expect(stored.get("agents-rule")!.copiedFromRuleId).toBeNull();
  });
});
