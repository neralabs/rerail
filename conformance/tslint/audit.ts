/**
 * Honesty audit of every rule whose effective verdict is PASS, kept in the pack as `packs/tslint/divergences.json`: does the port implement TSLint's semantics (for the
 * default options) on real code, or does it pass only because of what the fixtures happen to contain?
 *
 * - `genuine`: implements the rule's semantics.
 * - `bounded`: genuine, but a recursive or unbounded walk is unrolled or enumerated to a fixed depth (`depth`).
 * - `fixture-fit`: passes only because the fixtures do not exercise what the port misses or over-approximates.
 * - `option-baked`: the default doc hardcodes a list that is a TSLint option (whose default differs).
 * - `requires-config`: the port may be faithful, but TSLint has no meaningful option-less behaviour (a mandatory
 *   option, or bare `true` is inert or an absurd blanket ban), so the default doc either bakes a fixture's options or
 *   cannot steer. Listed with the TSLint evidence in `promote.ts`'s `REQUIRES_CONFIGURATION`; never promoted.
 *
 * - `poisoned`: passes because it matches identifiers only the fixtures use (`foo`, `MyClass`) rather than the structure
 *   the rule is about; the score moves when the harness's `--rename-identifiers` gate renames user identifiers.
 *
 * `primitive` names what would make a non-genuine port genuine; it feeds the primitive histogram.
 */

import { readFileSync } from "node:fs";
import { join } from "node:path";
import { z } from "zod";
import { TSLINT_PACK } from "./fixtures.js";

export const AUDIT_FILE = join(TSLINT_PACK, "divergences.json");
const AUDIT_FORMAT = "rerail-tslint-audit/v1";

export const PASS_CLASSES = ["genuine", "bounded", "fixture-fit", "option-baked", "requires-config", "poisoned"] as const;
export type PassClass = (typeof PASS_CLASSES)[number];

const passAuditSchema = z.strictObject({
  class: z.enum(PASS_CLASSES),
  depth: z.string().min(1).optional(),
  primitive: z.string().min(1).optional(),
  note: z.string().min(1),
});
export type PassAudit = z.infer<typeof passAuditSchema>;

const auditSchema = z.strictObject({ format: z.literal(AUDIT_FORMAT), rules: z.record(z.string(), passAuditSchema) });

export function parseAudit(text: string): Readonly<Record<string, PassAudit>> {
  return auditSchema.parse(JSON.parse(text)).rules;
}

export const PASS_AUDIT: Readonly<Record<string, PassAudit>> = parseAudit(readFileSync(AUDIT_FILE, "utf8"));
