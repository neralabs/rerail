/**
 * The React pack's own architecture rules (packs/react/conformance.json `own`) on the cases they were drafted and
 * reviewed with: each caught case must fire the rule and each passed case must not. The cases run as `rerail rules
 * test` runs a draft's cases (`testRuleDraftCases`), one file at its path. They are pack test data, not in git:
 * packs/react/upstream/react-architecture/<rule id>.cases.json, from that tool-pack's archive (`npm run fixtures:pull`),
 * as a draft's `cases` holds them.
 */
import { readFileSync } from "node:fs";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { requirePackFixture } from "../../scripts/s3/pack-fixtures.mjs";
import { parseRuleDraft, testRuleDraftCases } from "../../src/cli/index.js";
import { repositoryPath } from "../support/oracles.js";

const TIMEOUT = 120_000;

const readJson = (path: string): unknown => JSON.parse(readFileSync(path, "utf8"));

const { own } = z.looseObject({ own: z.array(z.string()) }).parse(readJson(repositoryPath("packs", "react", "conformance.json")));

describe("react pack architecture rules on their cases", () => {
  it("lists rules", () => {
    expect(own.length).toBeGreaterThan(0);
  });

  it.each(own)("%s", { timeout: TIMEOUT }, async id => {
    const rule = readJson(repositoryPath("packs", "react", "rules", `${id}.json`));
    const cases = readJson(requirePackFixture(repositoryPath("packs", "react", "upstream", "react-architecture", `${id}.cases.json`)));
    const draft = parseRuleDraft({ kind: "code", rule, cases });

    const results = await testRuleDraftCases(draft, repositoryPath());

    expect(results.some(result => result.expected === "caught")).toBe(true);
    expect(results.some(result => result.expected === "passed")).toBe(true);
    expect(results.filter(result => !result.ok)).toEqual([]);
  });
});
