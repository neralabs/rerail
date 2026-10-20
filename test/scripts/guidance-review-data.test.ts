import { describe, expect, it } from "vitest";
import { reviewData } from "../../scripts/review/guidance-review-data.js";

describe("guidance review data", () => {
  it("runs each complexity rule on its pack.json examples and renders the steer the coding agent reads", async () => {
    const data = await reviewData(["complexity"]);
    const [pack] = data.packs;
    expect(pack?.rules.map(rule => rule.id).sort()).toEqual(["complexity-cyclomatic-javascript", "complexity-cyclomatic-python", "complexity-cyclomatic-typescript"]);
    for (const rule of pack?.rules ?? []) {
      const caught = rule.cases.find(item => item.kind === "example");
      expect(caught?.guidance).toMatch(new RegExp(`^◆ Rerail · ${rule.id} {2}steered\\nFrom the Cyclomatic complexity \\(limit 6\\) rule pack\\.`, "u"));
      expect(rule.cases.find(item => item.kind === "passes")?.guidance).toBe("");
      expect(rule.digest).toMatch(/^[0-9a-f]{16}$/u);
    }
  });
});
