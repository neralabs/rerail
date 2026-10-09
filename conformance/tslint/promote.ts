/**
 * Promote a proven TSLint port into the golden set: Rerail's TSLint pack
 * (`packs/tslint/rules/<rule>.json`, rule id `tslint:<rule>`, seeded disabled). A port
 * qualifies when its PASS audit class is `genuine` (`packs/tslint/divergences.json`), TSLint gives it meaningful
 * option-less behaviour (not in `REQUIRES_CONFIGURATION`), and, scored with the doc as it would ship, its effective
 * verdict is PASS (raw or on a repaired fixture) and it scores identically on renamed fixtures.
 *
 * The shipped doc is the port's default-options doc with two changes: a description naming the TSLint rule and its
 * page, and a source-tree scope (TypeScript files, no dependency, build-output or generated directories). Clauses,
 * violations, anchors and fixes are the port's. Promotion writes the doc and the rule's conformance.json entry from the run,
 * then names the port default and verdict sidecar to `git rm`: the pack doc replaces them, and option-variant
 * overrides stay in `packs/tslint/candidates/<rule>/`. The rule's `pack.json` entry (title, group) is written by hand.
 *
 *   node --import tsx conformance/tslint/promote.ts <rule>    promote one port (needs the fixtures: `cli.ts fetch`)
 *   node --import tsx conformance/tslint/promote.ts --check   pack <-> conformance.json <-> audit consistency (no fixtures needed)
 */
import { existsSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { join, relative } from "node:path";
import { parseArgs } from "node:util";
import { pathToFileURL } from "node:url";
import { codeRuleSchema, type CodeRuleScope } from "../../src/code-rule-schema.js";
import { PASS_AUDIT } from "./audit.js";
import { ensureFixtures, loadScoredRules, RERAIL_ROOT } from "./fixtures.js";
import {
  GOLDEN_MANIFEST,
  goldenDocumentPath,
  goldenManifestText,
  goldenRules,
  isGoldenRule,
  loadGoldenManifest,
  membershipProblems,
  PACK_DIRECTORY,
  type GoldenEntry,
  type GoldenManifest,
} from "./golden.js";
import { observedEntry } from "./golden-gate.js";
import { renameCheck } from "./poison.js";
import { COMMITTED_PORTS_DIRECTORY, evaluateRule, loadSidecar } from "./runner.js";

/** TypeScript sources only: the engine parses `.js` as `javascript`, which no port's typescript AST query matches. */
export const INCLUDE = ["**/*.ts", "**/*.tsx", "**/*.mts", "**/*.cts"];
export const EXCLUDE = [
  "**/node_modules/**",
  "**/dist/**",
  "**/build/**",
  "**/out/**",
  "**/.next/**",
  "**/coverage/**",
  "**/generated/**",
  "**/__generated__/**",
  "**/*.generated.ts",
  "**/*.generated.tsx",
  "**/*.gen.ts",
];

/**
 * Rules TSLint gives no meaningful option-less behaviour: a mandatory option, or bare `true` is inert or an absurd
 * blanket ban. The port's default doc then either bakes a fixture's options or steers nothing, so the rule is never
 * promoted (a user who wants one configures it from the port's overrides). Each is audited `requires-config`.
 * Evidence is TSLint's source at the pinned SHA (tmp/tslint-fixtures/src/rules).
 */
export const REQUIRES_CONFIGURATION: ReadonlyMap<string, string> = new Map([
  ["array-type", "one of array/generic/array-simple must be provided (arrayTypeRule.ts:34); bare `true` flags both T[] and Array<T> (arrayTypeRule.ts:77, :101)"],
  ["comment-type", "the options list the allowed comment kinds (commentTypeRule.ts:44); bare `true` allows none, so every comment is flagged (commentTypeRule.ts:74-85)"],
  ["file-header", "the header regex is mandatory (fileHeaderRule.ts:52, minLength 1 at :94); the port's default doc bakes the fixtures' `Good header \\d` and flags every file"],
  ["max-classes-per-file", "the class limit is a required argument (maxClassesPerFileRule.ts:39, minLength 1 at :55); bare `true` compares against undefined and never fires (:70-71, :89)"],
  ["member-ordering", "an order object must be provided (memberOrderingRule.ts:164); bare `true` throws \"Got empty options\" and reports nothing (:306-310, :617-618)"],
  ["typedef", "every check is an opt-in argument (typedefRule.ts:50-56, :63; minLength 0 at :93); bare `true` checks nothing, so the default doc is a placeholder that cannot match"],
]);

export const descriptionPrefix = (rule: string): string =>
  `Port of TSLint's \`${rule}\` rule with its default options (https://palantir.github.io/tslint/rules/${rule}/).`;

function shippedScope(port: CodeRuleScope | undefined): CodeRuleScope {
  const portExcludes = (port?.exclude ?? []).filter(glob => !EXCLUDE.includes(glob));
  return { include: INCLUDE, exclude: [...EXCLUDE, ...portExcludes] };
}

/** The shipped doc for a port default: its JSON as written (key order kept), description and scope replaced. */
export function shippedDocument(rule: string, portPath = join(COMMITTED_PORTS_DIRECTORY, `${rule}.json`)): Record<string, unknown> {
  const raw: unknown = JSON.parse(readFileSync(portPath, "utf8"));
  const port = codeRuleSchema.parse(raw);
  const expectedId = `tslint:${rule}`;
  if (port.id !== expectedId) throw new Error(`port ${rule} has id ${port.id}, expected ${expectedId}`);
  const summary = port.description === undefined ? "" : ` ${port.description}`;
  const document = { ...(raw as Record<string, unknown>), description: `${descriptionPrefix(rule)}${summary}`, scope: shippedScope(port.scope) };
  codeRuleSchema.parse(document);
  return document;
}

export function documentText(document: Record<string, unknown>): string {
  return `${JSON.stringify(document, null, 2)}\n`;
}

/** Why `rule` may not be golden by audit: not audited `genuine`, or needs configuration. */
function auditProblems(rule: string): string[] {
  const problems: string[] = [];
  const audit = PASS_AUDIT[rule];
  if (audit?.class !== "genuine") problems.push(`${rule}: audit class ${audit?.class ?? "none"}, golden rules must be genuine (packs/tslint/divergences.json)`);
  if (REQUIRES_CONFIGURATION.has(rule)) problems.push(`${rule}: requires configuration (${REQUIRES_CONFIGURATION.get(rule)})`);
  return problems;
}

/**
 * Static consistency of the golden set: pack folder, conformance.json and audit agree, every pack doc is well-formed, and no
 * golden rule keeps a port default or verdict sidecar in `candidates/`.
 */
export function checkGolden(manifest: GoldenManifest = loadGoldenManifest(), directory = PACK_DIRECTORY): string[] {
  const rules = goldenRules(directory);
  const scored = new Set(loadScoredRules().map(rule => rule.name));
  const problems = membershipProblems(manifest, rules);
  for (const rule of rules) {
    if (!scored.has(rule)) problems.push(`${rule}: not a scored TSLint core rule`);
    problems.push(...auditProblems(rule));
    for (const stale of [`${rule}.json`, `${rule}.verdict.json`]) {
      const path = join(COMMITTED_PORTS_DIRECTORY, stale);
      if (existsSync(path)) problems.push(`${rule}: golden, so ${relative(RERAIL_ROOT, path)} duplicates the pack doc and conformance.json; git rm it`);
    }
    const entry = manifest.rules[rule];
    if (entry?.verdict === "PASS (repaired fixture)" && (entry.evidence ?? []).length === 0) {
      problems.push(`${rule}: a repaired-fixture PASS must cite the invalid fixture lines in conformance.json evidence`);
    }
    const parsed = codeRuleSchema.safeParse(JSON.parse(readFileSync(goldenDocumentPath(rule, directory), "utf8")));
    if (!parsed.success) {
      problems.push(`${rule}: pack doc is not a valid policy: ${parsed.error.issues[0]?.message ?? "invalid"}`);
      continue;
    }
    const document = parsed.data;
    if (document.id !== `tslint:${rule}`) problems.push(`${rule}: pack doc id ${document.id}, expected tslint:${rule}`);
    if (!(document.description ?? "").startsWith(descriptionPrefix(rule))) problems.push(`${rule}: pack doc description must start with "${descriptionPrefix(rule)}"`);
    const scope = document.scope;
    const include = JSON.stringify(scope?.include ?? []);
    const exclude = scope?.exclude ?? [];
    if (include !== JSON.stringify(INCLUDE) || JSON.stringify(exclude.slice(0, EXCLUDE.length)) !== JSON.stringify(EXCLUDE)) {
      problems.push(`${rule}: pack doc scope must include ${INCLUDE.join(" ")} and exclude ${EXCLUDE.join(" ")} first`);
    }
  }
  return problems;
}

/** Promote `rule`; returns the problems that stopped it (empty on success). */
export async function promote(rule: string, log: (line: string) => void = console.log): Promise<string[]> {
  if (process.env.RERAIL_TSLINT_PORTS !== undefined) return ["unset RERAIL_TSLINT_PORTS: promotion reads the committed ports"];
  const info = loadScoredRules().find(candidate => candidate.name === rule);
  if (info === undefined) return [`${rule}: not a scored TSLint core rule`];
  if (isGoldenRule(rule)) return [`${rule}: already golden`];
  const portPath = join(COMMITTED_PORTS_DIRECTORY, `${rule}.json`);
  if (!existsSync(portPath)) return [`${rule}: no port default at ${relative(RERAIL_ROOT, portPath)}`];
  const problems = auditProblems(rule);
  if (problems.length > 0) return problems;

  ensureFixtures(log);
  const packPath = goldenDocumentPath(rule);
  writeFileSync(packPath, documentText(shippedDocument(rule, portPath)));
  const result = await evaluateRule(info, 1);
  const observed = observedEntry(result);
  const rename = await renameCheck(info);
  if ("problem" in observed) problems.push(`${rule}: ${observed.problem} with the shipped doc (node --import tsx conformance/tslint/cli.ts --rule ${rule})`);
  if (rename.changed || rename.renamerIssues.length > 0) problems.push(`${rule}: scores differently on renamed fixtures (node --import tsx conformance/tslint/cli.ts --rename-identifiers --rule ${rule})`);
  if ("problem" in observed || problems.length > 0) {
    rmSync(packPath);
    return problems;
  }

  const sidecarPath = join(COMMITTED_PORTS_DIRECTORY, `${rule}.verdict.json`);
  const { sidecar } = loadSidecar(existsSync(sidecarPath) ? sidecarPath : undefined);
  const entry: GoldenEntry = {
    ...observed.entry,
    ...(sidecar === undefined
      ? {}
      : {
          reason: sidecar.reason,
          ...(sidecar.primitive === undefined ? {} : { primitive: sidecar.primitive }),
          ...(sidecar.secondary === undefined ? {} : { secondary: sidecar.secondary }),
          ...(sidecar.evidence.length === 0 ? {} : { evidence: sidecar.evidence }),
        }),
  };
  const manifest = loadGoldenManifest();
  writeFileSync(GOLDEN_MANIFEST, goldenManifestText({ ...manifest, rules: { ...manifest.rules, [rule]: entry } }));
  log(`promoted ${rule}: ${relative(RERAIL_ROOT, packPath)} (${entry.verdict}, TP ${entry.truePositives}${entry.outOfScope === undefined ? "" : `, out of scope ${entry.outOfScope.join(", ")}`})`);
  const stale = [portPath, sidecarPath].filter(path => existsSync(path)).map(path => relative(RERAIL_ROOT, path));
  log(`the pack doc replaces the port default; remove it with: git rm ${stale.join(" ")}`);
  log(`list it in ${relative(RERAIL_ROOT, join(PACK_DIRECTORY, "..", "pack.json"))}: { "id": "tslint:${rule}", "document": "rules/${rule}.json", "title": ..., "group": "TSLint" }`);
  if (entry.verdict === "PASS (repaired fixture)" && entry.evidence === undefined) log(`add evidence for the invalid fixture lines to ${relative(RERAIL_ROOT, GOLDEN_MANIFEST)}`);
  return [];
}

async function main(): Promise<void> {
  const { values, positionals } = parseArgs({ allowPositionals: true, options: { check: { type: "boolean", default: false } } });
  const problems = values.check ? checkGolden() : positionals.length === 1 ? await promote(positionals[0]!) : ["usage: node --import tsx conformance/tslint/promote.ts <rule> | --check"];
  if (values.check && problems.length === 0) console.log(`golden set consistent: ${goldenRules().length} rules in ${relative(RERAIL_ROOT, PACK_DIRECTORY)} and ${relative(RERAIL_ROOT, GOLDEN_MANIFEST)}`);
  for (const problem of problems) console.error(problem);
  if (problems.length > 0) process.exitCode = 1;
}

if (process.argv[1] !== undefined && import.meta.url === pathToFileURL(process.argv[1]).href) {
  main().catch((error: unknown) => {
    console.error(error instanceof Error ? error.message : error);
    process.exitCode = 1;
  });
}
