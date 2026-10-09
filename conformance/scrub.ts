/**
 * `packs/<id>/scrub.json`: the failures a pack's existing rules are known to have, one `(rule, sub-check)` entry each,
 * to fix or remove before the pack goes live. A covered failure leaves its rule `scrubbed` instead of `failed`. An
 * entry whose rule passes the sub-check is stale and fails it. `scripts/check-scrub.mjs` lets the list only shrink
 * against a pull request's base. Validation material: never published.
 */
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { z } from "zod";
import { parseJsonWithoutDuplicateKeys } from "../src/json-duplicate-keys.js";
import { compareDeterministicStrings } from "../src/stable-json.js";
import { SCRUBBABLE_SUB_CHECK_IDS } from "./checks.js";

export const SCRUB_FILE = "scrub.json";

export const scrubEntrySchema = z.strictObject({
  /** The rule's id, as pack.json lists it. */
  rule: z.string().min(1),
  /** A sub-check scrub.json may cover. */
  check: z.enum(SCRUBBABLE_SUB_CHECK_IDS),
});
export type ScrubEntry = z.infer<typeof scrubEntrySchema>;

const compareEntries = (left: ScrubEntry, right: ScrubEntry): number =>
  compareDeterministicStrings(left.rule, right.rule) || compareDeterministicStrings(left.check, right.check);

/** Entries ordered by rule, then sub-check, each once. */
export const scrubSchema = z.array(scrubEntrySchema).superRefine((entries, context) => {
  const index = entries.findIndex((entry, at) => at > 0 && compareEntries(entries[at - 1]!, entry) >= 0);
  if (index !== -1) context.addIssue({ code: "custom", message: `entry ${index} (${entries[index]!.rule}, ${entries[index]!.check}) is repeated or out of order` });
});

/** The pack's scrub list, empty when it has none; throws on a repeated key, a schema error, or a rule pack.json does not list. */
export function loadScrub(directory: string, rules: ReadonlySet<string>): ScrubEntry[] {
  const path = join(directory, SCRUB_FILE);
  if (!existsSync(path)) return [];
  const parsed = scrubSchema.safeParse(parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), SCRUB_FILE));
  if (!parsed.success) throw new Error(`${SCRUB_FILE}: ${z.prettifyError(parsed.error)}`);
  const unknown = [...new Set(parsed.data.map(entry => entry.rule).filter(rule => !rules.has(rule)))];
  if (unknown.length > 0) throw new Error(`${SCRUB_FILE}: ${unknown.join(", ")} not listed in pack.json`);
  return parsed.data;
}
