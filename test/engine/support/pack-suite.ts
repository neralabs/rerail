/**
 * Each pack's validation as tests (docs/pack-format.md, "The validation contract"): `describe("pack <id>")`, one test
 * per check and one per pack.json rule, a rule core cannot read yet skipped as not supported yet. Check 4 scores ported
 * rules on the tools' fixtures, unless `RERAIL_PACK_FIXTURES_IN_CONFORMANCE=1`: CI's unit tests set it, since the
 * conformance job scores the same fixtures and fails on the same failures, so the fixtures are scored once.
 *
 * A rule written in `.rerail` takes seconds to validate, where a JSON rule takes milliseconds: the engine compiles it
 * and runs it over each example's repository and its `.test` blocks' cases. In one test file the packs holding such
 * rules took 13 minutes on CI's Windows runner, past its unit shard's 15 with the shard's set-up, and CI's shards split
 * by test file. So those packs are validated in `RERAIL_PACK_FILES` files of their own (packs-rerail/share-<n>.test.ts),
 * each a share balanced by their count of `.rerail` rules, which the time-balanced shards place apart; every other
 * pack is validated in packs.test.ts. `npx vitest run test/engine/catalog/packs -t "pack <id>"` runs one pack's,
 * wherever it is.
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it, test } from "vitest";
import { z } from "zod";
import { PACK_CHECKS, validatePack, type PackValidation } from "../../../conformance/pack-validation.js";
import { compareDeterministicStrings } from "../../../src/engine/stable-json.js";

export const PACKS = fileURLToPath(new URL("../../../packs/", import.meta.url));
export const TIMEOUT = 600_000;
const FIXTURES_IN_CONFORMANCE = process.env.RERAIL_PACK_FIXTURES_IN_CONFORMANCE === "1";

/** How many test files (test/engine/catalog/packs-rerail/share-<n>.test.ts) validate the packs with `.rerail` rules. */
export const RERAIL_PACK_FILES = 4;

/** Every pack directory under packs/, by id. */
export const packs: readonly string[] = existsSync(PACKS)
  ? readdirSync(PACKS, { withFileTypes: true })
      .filter(entry => entry.isDirectory())
      .map(entry => entry.name)
      .sort(compareDeterministicStrings)
  : [];

const packJson = z.object({ rules: z.array(z.object({ id: z.string(), document: z.string() })) });

/** The rules pack.json lists, read when tests are collected; an unreadable pack.json lists none and fails check 1. */
function listedRules(id: string): { id: string; document: string }[] {
  try {
    const parsed = packJson.safeParse(JSON.parse(readFileSync(join(PACKS, id, "pack.json"), "utf8")));
    return parsed.success ? parsed.data.rules : [];
  } catch {
    return [];
  }
}

/** How many of the rules pack.json lists are written in `.rerail`. */
export function rerailRuleCount(id: string): number {
  return listedRules(id).filter(rule => rule.document.endsWith(".rerail")).length;
}

/**
 * The share of the packs with rules written in `.rerail` that file `file` (1 to `RERAIL_PACK_FILES`) validates: the
 * packs, most such rules first and then by id, each to the share with the fewest rules so far, the lowest on a tie.
 */
export function rerailPackShare(file: number): string[] {
  const counted = packs.map(id => ({ id, rules: rerailRuleCount(id) })).filter(pack => pack.rules > 0);
  counted.sort((a, b) => b.rules - a.rules || compareDeterministicStrings(a.id, b.id));
  const loads = Array.from({ length: RERAIL_PACK_FILES }, () => 0);
  const shares: string[][] = loads.map(() => []);
  for (const pack of counted) {
    const lightest = loads.indexOf(Math.min(...loads));
    loads[lightest]! += pack.rules;
    shares[lightest]!.push(pack.id);
  }
  return (shares[file - 1] ?? []).sort(compareDeterministicStrings);
}

/** Each of `ids` validated: `describe("pack <id>")`, a test per check and per rule pack.json lists. */
export function describePacks(ids: readonly string[]): void {
  describe.each(ids)("pack %s", id => {
    let validation: Promise<PackValidation> | undefined;
    const validated = (): Promise<PackValidation> => (validation ??= validatePack(join(PACKS, id), { fixtures: !FIXTURES_IN_CONFORMANCE }));

    it.each(PACK_CHECKS)("check $number: $name", { timeout: TIMEOUT }, async ({ name }) => {
      expect((await validated()).checks[name]).toEqual([]);
    });

    test.for(listedRules(id).map(rule => rule.id))("rule %s", { timeout: TIMEOUT }, async (ruleId, { skip }) => {
      const rule = (await validated()).rules.find(entry => entry.id === ruleId);
      if (rule?.status === "not-supported-yet") skip(`not supported yet: ${rule.reason}`);
      expect(rule?.status, rule?.reason).toBe("ok");
    });
  });
}
