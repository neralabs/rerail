import { afterEach, describe, expect, it } from "vitest";
import { ruleTitle } from "../../ui/derive.js";
import { data, titleOf } from "../../ui/state.js";

const MESSAGE = "Take this shadow off; use a 1 px border if it needs an edge.";

/** A stored code rule as the dashboard holds one, its first clause's message the agent's steer. */
const codeRule = (rule: Record<string, unknown>) => ({
  kind: "code",
  rule: { id: "local:no-shadows", mode: "steer", clauses: [{ id: "c", violation: { message: MESSAGE } }], ...rule },
});

afterEach(() => {
  data.packs = [];
});

describe("the name a rule's row and panel lead with", () => {
  it("is the rule's own title, over its pack's title, its first message and its id", () => {
    data.packs = [{ id: "design", rules: [{ id: "design-no-shadows", title: "No shadows (pack)" }] }];

    expect(titleOf(codeRule({ title: "No shadows except the one sheet shadow" }))).toBe("No shadows except the one sheet shadow");
    // A copy of a pack's rule keeps the title it was saved with.
    expect(titleOf(codeRule({ id: "design-no-shadows", title: "Ours" }))).toBe("Ours");
    expect(titleOf(codeRule({ id: "design-no-shadows" }))).toBe("No shadows (pack)");
  });

  it("falls back to the first message for a rule stored before titles, and to the id without one", () => {
    expect(titleOf(codeRule({}))).toBe(MESSAGE);
    expect(ruleTitle({ kind: "code", rule: { id: "local:empty", clauses: [] } })).toBe("local:empty");
  });
});
