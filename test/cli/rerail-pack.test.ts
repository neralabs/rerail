/**
 * Every rule of the rerail pack (packs/rerail/) on the cases it was drafted and reviewed with: each caught case must
 * fire the rule and each passed case must not. The cases run as `rerail rules test` runs a draft's cases
 * (`testRuleDraftCases`): a code rule's case is one file at its path, its imports resolved against this repository; a
 * conversation rule's case is one sample through the conversation engine. The cases are in
 * fixtures/rerail/<rule id>.cases.json, as a draft's `cases` holds them.
 */
import { readFileSync } from "node:fs";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { parseRuleDraft, testRuleDraftCases } from "../../src/cli/index.js";
import { fixturesPath, repositoryPath } from "../support/oracles.js";

const TIMEOUT = 120_000;

const readJson = (path: string): unknown => JSON.parse(readFileSync(path, "utf8"));

const packSchema = z.looseObject({ rules: z.array(z.looseObject({ id: z.string(), document: z.string() })) });
const { rules } = packSchema.parse(readJson(repositoryPath("packs", "rerail", "pack.json")));

/** A code rule names its format; a conversation rule has none. */
const kindOf = (rule: unknown): "code" | "conversation" =>
  typeof rule === "object" && rule !== null && "format" in rule ? "code" : "conversation";

describe("rerail pack rules on their cases", () => {
  it("lists rules", () => {
    expect(rules.length).toBeGreaterThan(0);
  });

  it.each(rules)("$id", { timeout: TIMEOUT }, async ({ id, document }) => {
    const rule = readJson(repositoryPath("packs", "rerail", document));
    const draft = parseRuleDraft({ kind: kindOf(rule), rule, cases: readJson(fixturesPath("rerail", `${id}.cases.json`)) });

    const results = await testRuleDraftCases(draft, repositoryPath());

    expect(results.some(result => result.expected === "caught")).toBe(true);
    expect(results.some(result => result.expected === "passed")).toBe(true);
    expect(results.filter(result => !result.ok)).toEqual([]);
  });
});
