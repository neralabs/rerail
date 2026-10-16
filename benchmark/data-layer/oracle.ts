// The data-layer linters' recorded findings (`tree.expected.json`, `mutations/`) and the sidecars of codes a port omits
// or matches only partly (`unsupported.json`), as `<oracles>/<linter>/` holds them.
import { existsSync, readFileSync, readdirSync, statSync } from "node:fs";
import { relative, resolve, sep } from "node:path";
import { z } from "zod";
import { compareStrings, type FindingKey } from "./engine.js";

const findingSchema = z.object({ code: z.string(), path: z.string(), line: z.number().int().nonnegative(), message: z.string().optional() });
export type OracleFinding = z.infer<typeof findingSchema>;
const treeSchema = z.object({ dataLayerCommit: z.string(), linter: z.string(), findings: z.array(findingSchema) });
const mutationExpectedSchema = z.object({
  dataLayerCommit: z.string(), linter: z.string(), mutation: z.string(),
  added: z.array(findingSchema), removed: z.array(findingSchema),
});
const manifestSchema = z.object({
  linter: z.string(),
  mutations: z.record(z.string(), z.object({ description: z.string(), files: z.array(z.string().min(1)).nonempty() })),
});

export interface Mutation {
  name: string;
  description: string;
  /** Repo-relative path → the file's full contents under the mutation. */
  files: Map<string, string>;
  added: OracleFinding[];
  removed: OracleFinding[];
}

export interface LinterOracle {
  linter: string;
  directory: string;
  dataLayerCommit: string;
  tree: OracleFinding[];
  mutations: Mutation[];
}

function readJson(path: string): unknown {
  return JSON.parse(readFileSync(path, "utf8")) as unknown;
}

function filesUnder(directory: string): string[] {
  return readdirSync(directory, { recursive: true, encoding: "utf8" })
    .filter(entry => statSync(resolve(directory, entry)).isFile())
    .map(entry => entry.split(sep).join("/"))
    .sort(compareStrings);
}

/** `<oracles>/<linter>/`, or `undefined` when the linter has no oracle; a malformed or unrecorded oracle throws. */
export function loadLinterOracle(oracles: string, linter: string): LinterOracle | undefined {
  const directory = resolve(oracles, linter);
  if (!existsSync(resolve(directory, "manifest.json"))) return undefined;
  const manifest = manifestSchema.parse(readJson(resolve(directory, "manifest.json")));
  if (manifest.linter !== linter) throw new Error(`${directory}/manifest.json names ${manifest.linter}, not ${linter}`);
  const treePath = resolve(directory, "tree.expected.json");
  if (!existsSync(treePath)) throw new Error(`${treePath} is not recorded; run generate.py ${linter}`);
  const tree = treeSchema.parse(readJson(treePath));
  const mutations = Object.entries(manifest.mutations).sort(([left], [right]) => compareStrings(left, right)).map(([name, entry]) => {
    const source = resolve(directory, "mutations", name);
    const listed = [...entry.files].sort(compareStrings);
    const present = existsSync(source) ? filesUnder(source) : [];
    if (JSON.stringify(listed) !== JSON.stringify(present)) {
      throw new Error(`${linter}/${name}: manifest lists ${JSON.stringify(listed)} but mutations/${name}/ holds ${JSON.stringify(present)}`);
    }
    const expectedPath = resolve(directory, "mutations", `${name}.expected.json`);
    if (!existsSync(expectedPath)) throw new Error(`${relative(oracles, expectedPath)} is not recorded; run generate.py ${linter}`);
    const expected = mutationExpectedSchema.parse(readJson(expectedPath));
    if (expected.dataLayerCommit !== tree.dataLayerCommit) {
      throw new Error(`${relative(oracles, expectedPath)} was recorded at ${expected.dataLayerCommit}, the tree at ${tree.dataLayerCommit}`);
    }
    return {
      name, description: entry.description,
      files: new Map(listed.map(path => [path, readFileSync(resolve(source, path), "utf8")])),
      added: expected.added, removed: expected.removed,
    };
  });
  return { linter, directory, dataLayerCommit: tree.dataLayerCommit, tree: tree.findings, mutations };
}

const countsSchema = z.object({ fp: z.number().int().nonnegative(), fn: z.number().int().nonnegative() });
const unsupportedSchema = z.object({
  linter: z.string(),
  codes: z.array(z.object({
    code: z.string().min(1),
    blockers: z.array(z.string().min(1)),
    reason: z.string().min(1),
    /** Present for a partial gap: the check stays in the policy and differs from the linter by exactly these counts. */
    mismatch: countsSchema.optional(),
    /**
     * Gaps only a mutation exposes, by mutation name: the code's difference in that mutation after the tree's own
     * differences are discounted. Unrecorded mutations expect 0/0.
     */
    mutations: z.record(z.string(), countsSchema).optional(),
  })),
});
export type UnsupportedCodes = z.infer<typeof unsupportedSchema>["codes"];

/** `<oracles>/<linter>/unsupported.json`: the finding codes the port omits (no `mismatch`) or matches only partly. */
export function loadUnsupported(oracles: string, linter: string): UnsupportedCodes {
  const path = resolve(oracles, linter, "unsupported.json");
  if (!existsSync(path)) return [];
  const sidecar = unsupportedSchema.parse(readJson(path));
  if (sidecar.linter !== linter) throw new Error(`${path} names ${sidecar.linter}, not ${linter}`);
  return sidecar.codes;
}

/** Mutation gaps the sidecar records for mutations the oracle does not have: a stale record. */
export function unknownSidecarMutations(unsupported: UnsupportedCodes, oracle: LinterOracle): string[] {
  const names = new Set(oracle.mutations.map(mutation => mutation.name));
  return unsupported.flatMap(entry => Object.keys(entry.mutations ?? {})
    .filter(name => !names.has(name))
    .map(name => `${entry.code}/${name}`));
}

export interface CodeResidual {
  code: string;
  fp: number;
  fn: number;
  recorded: { fp: number; fn: number };
  /** The observed residual equals the recorded one (0/0 when nothing is recorded). */
  matches: boolean;
}

/**
 * Judges one mutation's residual differences (after the tree's own are discounted) code by code: the mutation passes
 * iff every code's fp/fn equal the sidecar's `mutations[name]` counts, 0/0 when not recorded. A recorded gap that no
 * longer occurs fails too, so an improvement forces the sidecar to be updated.
 */
export function judgeMutation(name: string, extra: readonly FindingKey[], missing: readonly FindingKey[], unsupported: UnsupportedCodes): { passed: boolean; codes: CodeResidual[] } {
  const recordedByCode = Object.fromEntries(unsupported.flatMap(entry =>
    entry.mutations?.[name] === undefined ? [] : [[entry.code, entry.mutations[name]]]));
  const codes = [...new Set([...extra, ...missing].map(finding => finding.code).concat(Object.keys(recordedByCode)))].sort(compareStrings);
  const residuals = codes.map(code => {
    const fp = extra.filter(finding => finding.code === code).length;
    const fn = missing.filter(finding => finding.code === code).length;
    const recorded = recordedByCode[code] ?? { fp: 0, fn: 0 };
    return { code, fp, fn, recorded, matches: fp === recorded.fp && fn === recorded.fn };
  });
  return { passed: residuals.every(residual => residual.matches), codes: residuals };
}

function keyString(finding: FindingKey): string {
  return JSON.stringify([finding.code, finding.path, finding.line]);
}

/** Multiset arithmetic over (code, path, line); messages never take part. */
export function subtract<T extends FindingKey>(from: readonly T[], remove: readonly FindingKey[]): T[] {
  const pending = new Map<string, number>();
  for (const finding of remove) pending.set(keyString(finding), (pending.get(keyString(finding)) ?? 0) + 1);
  return from.filter(finding => {
    const count = pending.get(keyString(finding)) ?? 0;
    if (count === 0) return true;
    pending.set(keyString(finding), count - 1);
    return false;
  });
}

/** What the linter reports with the mutation applied: tree − removed + added. */
export function expectedAfter(tree: readonly OracleFinding[], mutation: Mutation): FindingKey[] {
  const missing = subtract(mutation.removed, tree);
  if (missing.length > 0) throw new Error(`${mutation.name} removes findings the tree does not hold: ${JSON.stringify(missing)}`);
  return [...subtract(tree, mutation.removed), ...mutation.added];
}

export function sortFindings<T extends FindingKey>(findings: readonly T[]): T[] {
  return [...findings].sort((left, right) => compareStrings(left.path, right.path) || left.line - right.line || compareStrings(left.code, right.code));
}

export interface Comparison {
  matched: number;
  /** In Rerail's findings only: false positives. */
  extra: FindingKey[];
  /** In the oracle only: false negatives. */
  missing: FindingKey[];
}

export function compareFindings(expected: readonly FindingKey[], actual: readonly FindingKey[]): Comparison {
  const extra = sortFindings(subtract(actual, expected));
  return { matched: actual.length - extra.length, extra, missing: sortFindings(subtract(expected, actual)) };
}
