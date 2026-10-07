/**
 * The golden set: the TSLint ports Rerail ships, disabled, in the opt-in pack
 * (`policies/executable-policy-dsl/rules/opt-in/tslint/<rule>.json`). The pack folder is the membership source of truth
 * and holds each golden rule's default-options doc; the harness scores those docs as shipped (their real-world scope),
 * pairing them with the rule's option-variant overrides in `rules/<rule>/<variant>.json`.
 *
 * `golden.json` replaces verdict sidecars for golden rules. Per rule it records what the gate asserts on every full run
 * and on `node --import tsx conformance/tslint/cli.ts golden`: the effective verdict (`PASS`, or `PASS (repaired fixture)` when invalid
 * fixture syntax is the only blocker), the true positives on the fixtures in scope (on the repaired fixtures for a
 * repaired-fixture PASS) with no false positives, false negatives or engine failures, and the fixture files the shipped
 * scope leaves out (`<variant>/<file>`, reported as out of scope, never as misses). `reason`, `primitive`, `secondary`
 * and `evidence` carry the notes a sidecar used to give the report; for a repaired-fixture PASS, `evidence` cites the
 * fixture lines that are not valid TypeScript.
 */
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { z } from "zod";
import { HARNESS_DIRECTORY, RERAIL_ROOT } from "./fixtures.js";
import { compareDeterministicStrings } from "../../src/stable-json.js";

export const PACK_DIRECTORY = join(RERAIL_ROOT, "policies", "executable-policy-dsl", "rules", "opt-in", "tslint");
export const GOLDEN_MANIFEST = join(HARNESS_DIRECTORY, "golden.json");
const GOLDEN_FORMAT = "rerail-tslint-golden/v1";

export const evidenceSchema = z.strictObject({
  variant: z.string().min(1),
  file: z.string().min(1).optional(),
  line: z.number().int().positive(),
  why: z.string().min(1),
});

export const goldenEntrySchema = z.strictObject({
  verdict: z.enum(["PASS", "PASS (repaired fixture)"]),
  truePositives: z.number().int().nonnegative(),
  outOfScope: z.array(z.string().min(1)).optional(),
  reason: z.string().min(1).optional(),
  primitive: z.string().min(1).optional(),
  secondary: z.array(z.string().min(1)).optional(),
  evidence: z.array(evidenceSchema).optional(),
});
export type GoldenEntry = z.infer<typeof goldenEntrySchema>;

const manifestSchema = z.strictObject({
  format: z.literal(GOLDEN_FORMAT),
  rules: z.record(z.string(), goldenEntrySchema),
});
export type GoldenManifest = z.infer<typeof manifestSchema>;

/** Whether `rule` is golden: its default doc is in the pack folder. */
export function isGoldenRule(rule: string, directory = PACK_DIRECTORY): boolean {
  return existsSync(join(directory, `${rule}.json`));
}

/** The golden rules: every `*.json` in the pack folder, sorted. */
export function goldenRules(directory = PACK_DIRECTORY): string[] {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true })
    .filter(entry => entry.isFile() && entry.name.endsWith(".json"))
    .map(entry => entry.name.slice(0, -".json".length))
    .sort(compareDeterministicStrings);
}

export function goldenDocumentPath(rule: string, directory = PACK_DIRECTORY): string {
  return join(directory, `${rule}.json`);
}

export function parseGoldenManifest(text: string): GoldenManifest {
  return manifestSchema.parse(JSON.parse(text));
}

let cached: GoldenManifest | undefined;
export function loadGoldenManifest(path = GOLDEN_MANIFEST): GoldenManifest {
  if (path !== GOLDEN_MANIFEST) return parseGoldenManifest(readFileSync(path, "utf8"));
  cached ??= parseGoldenManifest(readFileSync(path, "utf8"));
  return cached;
}

/** Rules sorted by name, so a promotion diff touches one entry. */
export function goldenManifestText(manifest: GoldenManifest): string {
  const rules = Object.fromEntries(Object.entries(manifest.rules).sort(([left], [right]) => compareDeterministicStrings(left, right)));
  return `${JSON.stringify({ format: manifest.format, rules }, null, 2)}\n`;
}

export function emptyGoldenManifest(): GoldenManifest {
  return { format: GOLDEN_FORMAT, rules: {} };
}

/** Membership: the manifest lists exactly the pack's rules. */
export function membershipProblems(manifest: GoldenManifest, rules = goldenRules()): string[] {
  const listed = new Set(Object.keys(manifest.rules));
  const packed = new Set(rules);
  return [
    ...rules.filter(rule => !listed.has(rule)).map(rule => `${rule}: in the pack but has no golden.json entry`),
    ...[...listed].filter(rule => !packed.has(rule)).sort(compareDeterministicStrings).map(rule => `${rule}: golden.json entry without a pack doc`),
  ];
}
