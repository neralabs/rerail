/**
 * The builder's model of a sample before it is packed (format.ts says what the archive holds): one file of one source
 * repository at one commit, its labels keyed by pack rule id, and the context files that ride along beside it.
 */
import { createHash } from "node:crypto";

import { LABELS_SUFFIX, SAMPLE_ID, sampleLabelsSchema, type SampleLabel, type SampleLabels } from "./format.js";

/** Code-unit order, the same on every machine (localeCompare depends on the ICU and locale). */
export const byText = (a: string, b: string): number => (a < b ? -1 : a > b ? 1 : 0);

export const sha256 = (bytes: Buffer | string): string => createHash("sha256").update(bytes).digest("hex");

/** A tool-pack: `packs/<pack>/upstream/<tool>/`, which owns one sample set. */
export interface ToolPack {
  pack: string;
  tool: string;
}

export const toolPackName = ({ pack, tool }: ToolPack): string => `${pack}/${tool}`;

/**
 * Where a sample's labels came from (a golden, a corpus recording, the ESLint test set, a replacement): provenance the
 * set was built from, which `dedupe` names in a label conflict.
 */
export type Origin = "golden" | "corpus" | "eslint test set" | "php test set" | "replacement";

export interface ContextFile {
  /** Repository-relative, posix. */
  path: string;
  bytes: Buffer;
}

export interface Sample {
  toolPack: ToolPack;
  /** The source's short name in the sample id: the repository's name, `-fixed` for an ESLint test-set fixed copy. */
  short: string;
  source: SampleLabels["source"];
  bytes: Buffer;
  labelledBy: SampleLabels["labelledBy"];
  /** Pack rule id -> labels; `[]` is clean, no key is not judged. */
  expected: ReadonlyMap<string, readonly SampleLabel[]>;
  /** Files the sample's repository holds beside it that change how it is read (Ruff's `__init__.py`, package.json). */
  context: readonly ContextFile[];
  /** Kept whatever its labels: the ESLint test set's files and the licence replacements are chosen samples already. */
  pinned: boolean;
  origins: readonly Origin[];
}

/** The identity samples are deduplicated by: the same bytes at the same place in the same repository. */
export const sampleKey = (sample: Pick<Sample, "source">): string => `${sample.source.repo}\0${sample.source.path}\0${sample.source.sha256}`;

/** The most a path slug keeps of its path, from the end (the file's name), so an id stays a short directory name. */
const PATH_SLUG_LIMIT = 80;

const slug = (text: string): string => text.toLowerCase().replace(/[^a-z0-9._-]+/gu, "-").replace(/-{2,}/gu, "-");

/** `<source>-<path slug>-<sha256[0:12]>`, lowercase: the same file of the same source always has the same id. */
export function sampleId(short: string, path: string, digest: string): string {
  const pathSlug = slug(path).slice(-PATH_SLUG_LIMIT).replace(/^[^a-z0-9]+/u, "");
  const id = `${slug(short).replace(/^[^a-z0-9]+/u, "")}-${pathSlug}-${digest.slice(0, 12)}`.replace(/-{2,}/gu, "-");
  if (!SAMPLE_ID.test(id)) throw new Error(`sample id ${id} (from ${short} ${path}) is not a valid sample id`);
  return id;
}

const byLabel = (a: SampleLabel, b: SampleLabel): number => a.line - b.line || (a.column ?? 0) - (b.column ?? 0);

/** The labels file's value: keys in schema order, rule ids sorted, each rule's labels by line then column. */
export function sampleLabels(sample: Sample): SampleLabels {
  const expected = Object.fromEntries(
    [...sample.expected.keys()].sort(byText).map((rule) => [rule, [...sample.expected.get(rule)!].sort(byLabel).map((label) => ({ ...label }))]),
  );
  return sampleLabelsSchema.parse({ source: sample.source, labelledBy: sample.labelledBy, expected });
}

export function labelsText(sample: Sample): string {
  return `${JSON.stringify(sampleLabels(sample), null, 2)}\n`;
}

/** A label disagreement two sources of one file had for one rule: the first source's labels are kept. */
export interface LabelConflict {
  key: string;
  rule: string;
  kept: Origin;
  dropped: Origin;
}

/** Labels agree when they mark the same lines: labels are line-only, columns are provenance. */
function labelsEqual(a: readonly SampleLabel[], b: readonly SampleLabel[]): boolean {
  const lines = (labels: readonly SampleLabel[]): string => labels.map((label) => label.line).sort((x, y) => x - y).join(",");
  return lines(a) === lines(b);
}

/**
 * One sample of the same file read from two sources: every rule either labels, the first's labels where both do (a
 * disagreement is returned), the first's provenance, and the context files of both.
 */
export function mergeSamples(first: Sample, second: Sample): { sample: Sample; conflicts: LabelConflict[]; added: string[] } {
  if (sampleKey(first) !== sampleKey(second) || toolPackName(first.toolPack) !== toolPackName(second.toolPack)) {
    throw new Error(`cannot merge ${sampleKey(first)} with ${sampleKey(second)}`);
  }
  const expected = new Map(first.expected);
  const conflicts: LabelConflict[] = [];
  const added: string[] = [];
  for (const [rule, labels] of second.expected) {
    const kept = expected.get(rule);
    if (kept === undefined) {
      expected.set(rule, labels);
      added.push(rule);
    } else if (!labelsEqual(kept, labels)) conflicts.push({ key: sampleKey(first), rule, kept: first.origins[0]!, dropped: second.origins[0]! });
  }
  const context = new Map(first.context.map((file) => [file.path, file]));
  for (const file of second.context) if (!context.has(file.path)) context.set(file.path, file);
  return {
    sample: {
      ...first,
      expected,
      context: [...context.values()].sort((a, b) => byText(a.path, b.path)),
      pinned: first.pinned || second.pinned,
      origins: [...new Set([...first.origins, ...second.origins])],
    },
    conflicts,
    added,
  };
}

/** Samples deduplicated by repository, path and sha256, merged in the order given (earlier sources win a disagreement). */
export function dedupe(samples: readonly Sample[]): { samples: Sample[]; conflicts: LabelConflict[]; merged: number } {
  const byKey = new Map<string, Sample>();
  const conflicts: LabelConflict[] = [];
  let merged = 0;
  for (const sample of samples) {
    const key = sampleKey(sample);
    const known = byKey.get(key);
    if (known === undefined) {
      byKey.set(key, sample);
      continue;
    }
    const result = mergeSamples(known, sample);
    byKey.set(key, result.sample);
    conflicts.push(...result.conflicts);
    merged += 1;
  }
  return { samples: [...byKey.values()].sort((a, b) => byText(sampleKey(a), sampleKey(b))), conflicts, merged };
}

/** The archive entries of a sample set (format.ts's layout), `LICENSES.md` first; ids are checked unique. */
export function archiveEntries(samples: readonly Sample[], licenses: string): { path: string; bytes: Buffer }[] {
  const entries: { path: string; bytes: Buffer }[] = [{ path: "LICENSES.md", bytes: Buffer.from(licenses, "utf8") }];
  const ids = new Map<string, string>();
  for (const sample of samples) {
    const id = sampleId(sample.short, sample.source.path, sample.source.sha256);
    const other = ids.get(id);
    if (other !== undefined) throw new Error(`samples ${other} and ${sampleKey(sample)} have the same id ${id}`);
    ids.set(id, sampleKey(sample));
    entries.push({ path: `${id}/${sample.source.path}`, bytes: sample.bytes });
    entries.push({ path: `${id}/${sample.source.path}${LABELS_SUFFIX}`, bytes: Buffer.from(labelsText(sample), "utf8") });
    for (const file of sample.context) {
      if (file.path === sample.source.path) continue;
      entries.push({ path: `${id}/${file.path}`, bytes: file.bytes });
    }
  }
  return entries;
}

/** Whether any rule labels a violation in the sample. */
export const hasPositive = (sample: Pick<Sample, "expected">): boolean => [...sample.expected.values()].some((labels) => labels.length > 0);
