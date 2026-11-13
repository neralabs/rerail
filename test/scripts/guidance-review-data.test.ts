import { describe, expect, it } from "vitest";
import { reviewData } from "../../scripts/review/guidance-review-data.js";

describe("guidance review data", () => {
  it("runs each complexity rule on its pack.json examples and renders the steer the coding agent reads", async () => {
    const data = await reviewData(["complexity"]);
    const [pack] = data.packs;
    expect(pack?.rules.map(rule => rule.id).sort()).toEqual([
      "complexity-cyclomatic-blade",
      "complexity-cyclomatic-java",
      "complexity-cyclomatic-javascript",
      "complexity-cyclomatic-php",
      "complexity-cyclomatic-python",
      "complexity-cyclomatic-rust",
      "complexity-cyclomatic-typescript",
      "complexity-cyclomatic-vue",
    ]);
    // The Java rule is a candidate: pack.json lists no examples for it, so its cases are its fixtures alone.
    expect(pack?.rules.filter(rule => rule.candidate).map(rule => rule.id)).toEqual(["complexity-cyclomatic-java"]);
    for (const rule of pack?.rules.filter(rule => !rule.candidate) ?? []) {
      const caught = rule.cases.find(item => item.kind === "example");
      // Verbatim what the agent reads after an edit: the lead, each finding with its location and rule, the latitude.
      // Blade's example is a closure, named by the variable it is assigned to.
      expect(caught?.guidance).toMatch(new RegExp(`^Rerail found 1 problem in your last edit:\\n- \\S+:\\d+:\\d+ — \`\\$?grade\` has cyclomatic complexity \\d+, over the ceiling of 6\\. .* \\[${rule.id}\\]\\nFix it, or tell the user in one line why you won't and run \`rerail feedback ${rule.id} '<that line>'\`\\.$`, "u"));
      expect(rule.cases.find(item => item.kind === "passes")?.guidance).toBe("");
      expect(rule.digest).toMatch(/^[0-9a-f]{16}$/u);
    }
  });
});
