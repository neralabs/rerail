// The translation suite's decisions (scripts/ship/translations.ts runs the engines): each rule's findings through its
// original JSON document and through its `.rerail` translation compared as sets, each rule's outcome, a pack's
// declaration (docs/rule-syntax/packs/<pack>/equivalence.json) read, checked against a run (the gate) and rewritten
// from one, and a pack's examples laid out as corpora. Nothing here runs an engine or touches the file system.

/** The declaration's format: equivalence.json's `format`. */
export const DECLARATION_FORMAT = "rerail.translation-equivalence/v1";

/** What --write-differences gives an entry nobody has explained yet, in a pack that does not gate. */
export const NOT_EXPLAINED = "recorded by the suite, not yet explained";

/** How many keys of each side's own a result keeps as samples, per corpus. */
export const SAMPLES = 5;

/** The corpus of a pack's own examples (pack.json `rules[].examples`). */
export const EXAMPLES_CORPUS = "examples";

/** The corpus of a pack's recorded upstream fixtures (the benchmark's prepared `recorded/<pack>/files`). */
export const FIXTURES_CORPUS = "fixtures";

/** One finding: the rule it is for, where, and its code; the column (1-based) when the side gives one. */
export interface Finding {
  rule: string;
  path: string;
  line: number;
  code: string;
  column?: number;
}

/** Who reports a difference correctly; `unknown` only in a pack that does not gate. */
export type Right = "original" | "translation" | "neither" | "unknown";
const RIGHTS: readonly Right[] = ["original", "translation", "neither", "unknown"];

export interface Counts {
  originalOnly: number;
  translationOnly: number;
}

export interface Difference {
  reason: string;
  right: Right;
  /** Per corpus, the keys only one side reports. */
  counts: Record<string, Counts>;
}

/** docs/rule-syntax/packs/<pack>/equivalence.json. */
export interface Declaration {
  format: typeof DECLARATION_FORMAT;
  pack: string;
  /** Every translated rule reports what its original does, except those in `different`: the benchmark gates on it. */
  equivalent: boolean;
  /** A rule without a runnable translation, with the engine's refusal or the feature it lacks. */
  untranslated: Record<string, string>;
  different: Record<string, Difference>;
  /** What a run that wrote this file could not compare (a corpus it could not run), in a pack that does not gate. */
  incomplete?: Record<string, string>;
}

/** One corpus's comparison of one rule: keys both report, and each side's own (count and samples). */
export interface CorpusComparison {
  both: number;
  originalOnly: number;
  translationOnly: number;
  samples: { originalOnly: string[]; translationOnly: string[] };
  /**
   * The original's failures on files of the corpus (a file with syntax errors, a work budget spent): the product
   * reports them and carries on, so the rule is compared still, and a difference on such a file may be theirs.
   */
  originalFailures?: { count: number; first: string };
}

export type Status = "same" | "different" | "untranslated" | "failed";
export const STATUSES: readonly Status[] = ["same", "different", "untranslated", "failed"];

export interface RuleResult {
  rule: string;
  status: Status;
  /** untranslated: why; failed: the engine's or the runner's message, with the side and corpus. */
  message?: string;
  /** Per corpus compared; empty for an untranslated or failed rule. */
  corpora: Record<string, CorpusComparison>;
}

/** A finding's identity on one side, without its column: rule, path, line and code. */
export function findingKey(f: Finding): string {
  return `${f.rule}\u0000${f.path}\u0000${f.line}\u0000${f.code}`;
}

/** A key as people read it: `path:line code`, with `:column` when one is compared. */
function shown(key: string, column?: number): string {
  const [, path, line, code] = key.split("\u0000");
  return `${path}:${line}${column === undefined ? "" : `:${column}`} ${code}`;
}

/** Each key's columns as the side gives them; a finding without one adds none. */
function byKey(findings: readonly Finding[]): Map<string, Set<number>> {
  const out = new Map<string, Set<number>>();
  for (const f of findings) {
    const key = findingKey(f);
    let columns = out.get(key);
    if (columns === undefined) out.set(key, (columns = new Set()));
    if (f.column !== undefined && f.column > 0) columns.add(f.column);
  }
  return out;
}

/**
 * Two sides' findings of one rule compared as sets of (rule, path, line, code), and by column too at a key where both
 * sides give columns: a column only one side reports is that side's own. Messages are not compared.
 */
export function compareFindings(original: readonly Finding[], translation: readonly Finding[]): CorpusComparison {
  const o = byKey(original);
  const t = byKey(translation);
  const originalOnly: string[] = [];
  const translationOnly: string[] = [];
  let both = 0;
  for (const [key, oc] of o) {
    const tc = t.get(key);
    if (tc === undefined) {
      originalOnly.push(shown(key));
      continue;
    }
    if (oc.size === 0 || tc.size === 0) {
      both += 1;
      continue;
    }
    for (const c of oc) if (tc.has(c)) both += 1;
    for (const c of oc) if (!tc.has(c)) originalOnly.push(shown(key, c));
    for (const c of tc) if (!oc.has(c)) translationOnly.push(shown(key, c));
  }
  for (const key of t.keys()) if (!o.has(key)) translationOnly.push(shown(key));
  originalOnly.sort();
  translationOnly.sort();
  return {
    both,
    originalOnly: originalOnly.length,
    translationOnly: translationOnly.length,
    samples: { originalOnly: originalOnly.slice(0, SAMPLES), translationOnly: translationOnly.slice(0, SAMPLES) },
  };
}

/** Each rule's findings, from a side's findings of every rule (rules without any get an empty list). */
export function byRule(findings: readonly Finding[]): Map<string, Finding[]> {
  const out = new Map<string, Finding[]>();
  for (const f of findings) {
    const list = out.get(f.rule);
    if (list === undefined) out.set(f.rule, [f]);
    else list.push(f);
  }
  return out;
}

/** What one rule's run gave, per corpus: both sides' findings, or why a side could not run it. */
export interface RuleInput {
  rule: string;
  /** The translation's file, or undefined when the pack has none for the rule. */
  translation: string | undefined;
  /** Why the translation cannot run before any corpus (no .policy line, check-rr's refusal), else undefined. */
  refused?: string;
  /**
   * Per corpus: each side's findings of this rule, with the original's failures on some files, or a failure that
   * leaves the rule without findings to compare (a side's run that failed, an original that does not compile).
   */
  corpora: Record<string, { original: readonly Finding[]; translation: readonly Finding[]; originalFailures?: readonly string[] } | { failure: string }>;
}

/** A rule's outcome: untranslated, failed (refused, or a side failed on a corpus), different, or same. */
export function ruleResult(input: RuleInput): RuleResult {
  if (input.translation === undefined) return { rule: input.rule, status: "untranslated", message: "no .rerail translation", corpora: {} };
  if (input.refused !== undefined) return { rule: input.rule, status: "failed", message: input.refused, corpora: {} };
  // One failure met on several corpora is said once, after the corpora it was met on.
  const failures = new Map<string, string[]>();
  for (const [corpus, run] of Object.entries(input.corpora)) if ("failure" in run) failures.set(run.failure, [...(failures.get(run.failure) ?? []), corpus]);
  if (failures.size > 0) return { rule: input.rule, status: "failed", message: [...failures].map(([why, where]) => `${where.join(", ")}: ${why}`).join("; "), corpora: {} };
  const corpora: Record<string, CorpusComparison> = {};
  for (const [corpus, run] of Object.entries(input.corpora)) {
    if ("failure" in run) continue;
    const c = compareFindings(run.original, run.translation);
    corpora[corpus] = run.originalFailures === undefined || run.originalFailures.length === 0 ? c : { ...c, originalFailures: { count: run.originalFailures.length, first: run.originalFailures[0]! } };
  }
  const differs = Object.values(corpora).some((c) => c.originalOnly + c.translationOnly > 0);
  return { rule: input.rule, status: differs ? "different" : "same", corpora };
}

/** Per status, how many rules have it. */
export function summarize(results: readonly RuleResult[]): Record<Status, number> {
  const out = { same: 0, different: 0, untranslated: 0, failed: 0 };
  for (const r of results) out[r.status] += 1;
  return out;
}

/** A rule's per-corpus counts of keys only one side reports, for the corpora where some are. */
export function countsOf(result: RuleResult): Record<string, Counts> {
  const out: Record<string, Counts> = {};
  for (const [corpus, c] of Object.entries(result.corpora).sort(([a], [b]) => a.localeCompare(b))) {
    if (c.originalOnly + c.translationOnly > 0) out[corpus] = { originalOnly: c.originalOnly, translationOnly: c.translationOnly };
  }
  return out;
}

// ---------------------------------------------------------------- the declaration

/** A declaration read from its JSON text, or the reason it is not one. */
export function parseDeclaration(text: string, pack: string): Declaration {
  const value: unknown = JSON.parse(text);
  const fail = (why: string): never => {
    throw new Error(`equivalence.json of ${pack}: ${why}`);
  };
  if (typeof value !== "object" || value === null || Array.isArray(value)) fail("not a JSON object");
  const d = value as Record<string, unknown>;
  if (d.format !== DECLARATION_FORMAT) fail(`format is ${JSON.stringify(d.format)}, not ${DECLARATION_FORMAT}`);
  if (d.pack !== pack) fail(`names pack ${JSON.stringify(d.pack)}`);
  if (typeof d.equivalent !== "boolean") fail("equivalent is not true or false");
  const record = (name: string): Record<string, unknown> => {
    const field = d[name];
    if (typeof field !== "object" || field === null || Array.isArray(field)) fail(`${name} is not an object`);
    return field as Record<string, unknown>;
  };
  const untranslated = record("untranslated");
  for (const [rule, why] of Object.entries(untranslated)) if (typeof why !== "string") fail(`untranslated.${rule} is not a text`);
  const different: Record<string, Difference> = {};
  for (const [rule, entry] of Object.entries(record("different"))) {
    if (typeof entry !== "object" || entry === null) fail(`different.${rule} is not an object`);
    const e = entry as Record<string, unknown>;
    if (typeof e.reason !== "string") fail(`different.${rule}.reason is not a text`);
    if (!RIGHTS.includes(e.right as Right)) fail(`different.${rule}.right is not one of ${RIGHTS.join(", ")}`);
    if (typeof e.counts !== "object" || e.counts === null || Array.isArray(e.counts)) fail(`different.${rule}.counts is not an object`);
    const counts: Record<string, Counts> = {};
    for (const [corpus, c] of Object.entries(e.counts as Record<string, unknown>)) {
      const n = c as Partial<Counts> | null;
      if (n === null || !Number.isInteger(n.originalOnly) || !Number.isInteger(n.translationOnly)) fail(`different.${rule}.counts.${corpus} is not { originalOnly, translationOnly }`);
      counts[corpus] = { originalOnly: n!.originalOnly!, translationOnly: n!.translationOnly! };
    }
    different[rule] = { reason: e.reason as string, right: e.right as Right, counts };
  }
  const declaration: Declaration = { format: DECLARATION_FORMAT, pack, equivalent: d.equivalent as boolean, untranslated: untranslated as Record<string, string>, different };
  if (d.incomplete !== undefined) {
    const incomplete = record("incomplete");
    for (const [corpus, why] of Object.entries(incomplete)) if (typeof why !== "string") fail(`incomplete.${corpus} is not a text`);
    declaration.incomplete = incomplete as Record<string, string>;
  }
  return declaration;
}

/** A pack's declaration before anyone wrote one: not equivalent, nothing listed. */
export function emptyDeclaration(pack: string): Declaration {
  return { format: DECLARATION_FORMAT, pack, equivalent: false, untranslated: {}, different: {} };
}

const sortedRecord = <T>(record: Record<string, T>): Record<string, T> => Object.fromEntries(Object.entries(record).sort(([a], [b]) => a.localeCompare(b)));

/** The declaration's JSON text, its lists sorted by rule. */
export function renderDeclaration(d: Declaration): string {
  const out: Record<string, unknown> = { format: d.format, pack: d.pack, equivalent: d.equivalent, untranslated: sortedRecord(d.untranslated), different: sortedRecord(d.different) };
  if (d.incomplete !== undefined && Object.keys(d.incomplete).length > 0) out.incomplete = sortedRecord(d.incomplete);
  return `${JSON.stringify(out, null, 2)}\n`;
}

const NONE: Counts = { originalOnly: 0, translationOnly: 0 };
const countsText = (c: Counts): string => `${c.originalOnly} original-only, ${c.translationOnly} translation-only`;

/**
 * Every way a run disagrees with the pack's declaration: a translated rule that differs and is not listed in
 * `different`, a listed rule whose counts changed on a corpus the run compared or that is the same now, an entry without
 * its reason, a rule without a translation not listed in `untranslated` (or listed and translated now), a rule that
 * failed, an entry naming no rule of the pack. `ran` names the corpora the run compared: counts of others are not
 * checked. Only a pack declaring `equivalent: true` fails on them (gates); the rest report them.
 */
export function gateProblems(declaration: Declaration, results: readonly RuleResult[], ran: readonly string[], pack: readonly string[]): string[] {
  const problems: string[] = [];
  const gating = declaration.equivalent;
  for (const r of results) {
    const listedDifferent = declaration.different[r.rule];
    const listedUntranslated = declaration.untranslated[r.rule];
    if (r.status === "failed") {
      problems.push(`${r.rule}: failed: ${r.message ?? ""}`);
      continue;
    }
    if (r.status === "untranslated") {
      if (listedUntranslated === undefined) problems.push(`${r.rule}: has no translation and is not listed in untranslated`);
      else if (gating && listedUntranslated.trim() === "") problems.push(`${r.rule}: listed in untranslated without the engine's refusal or the feature it lacks`);
      continue;
    }
    if (listedUntranslated !== undefined) problems.push(`${r.rule}: is translated now; remove it from untranslated`);
    if (r.status === "same") {
      if (listedDifferent !== undefined) problems.push(`${r.rule}: reports the same findings as its original now; remove it from different`);
      continue;
    }
    const counts = countsOf(r);
    if (listedDifferent === undefined) {
      const where = Object.entries(counts).map(([corpus, c]) => `${corpus}: ${countsText(c)}`).join("; ");
      problems.push(`${r.rule}: reports different findings and is not listed in different (${where})`);
      continue;
    }
    if (gating && listedDifferent.reason.trim() === "") problems.push(`${r.rule}: listed in different without a reason`);
    if (gating && listedDifferent.right === "unknown") problems.push(`${r.rule}: listed in different without saying which side is right (original, translation or neither)`);
    for (const corpus of ran) {
      const declared = listedDifferent.counts[corpus] ?? NONE;
      const now = counts[corpus] ?? NONE;
      if (declared.originalOnly !== now.originalOnly || declared.translationOnly !== now.translationOnly) problems.push(`${r.rule}: on ${corpus}, declared ${countsText(declared)}, now ${countsText(now)}`);
    }
  }
  const known = new Set(pack);
  for (const rule of Object.keys(declaration.different)) if (!known.has(rule)) problems.push(`${rule}: listed in different, but the pack has no such rule`);
  for (const rule of Object.keys(declaration.untranslated)) if (!known.has(rule)) problems.push(`${rule}: listed in untranslated, but the pack has no such rule`);
  return problems;
}

/**
 * The declaration rewritten from a run (--write-differences): `different` and `untranslated` hold the run's different
 * and untranslated rules, an entry already there keeping its reason and the counts of corpora the run did not compare.
 * A new entry is explained as NOT_EXPLAINED, right `unknown`, in a pack that does not gate; in one that gates it is
 * left empty, so the gate fails until someone explains it. A rule that failed keeps whatever entry it had; in a pack
 * that does not gate, one without an entry is listed untranslated with its failure. `incomplete` replaces the
 * file's (a gating pack's run is complete or fails).
 */
export function writeDifferences(declaration: Declaration, results: readonly RuleResult[], ran: readonly string[], incomplete: Record<string, string> = {}): Declaration {
  const gating = declaration.equivalent;
  const untranslated: Record<string, string> = {};
  const different: Record<string, Difference> = {};
  for (const r of results) {
    const before = declaration.different[r.rule];
    if (r.status === "untranslated") {
      untranslated[r.rule] = declaration.untranslated[r.rule] ?? (gating ? "" : `${r.message ?? "no .rerail translation"}; ${NOT_EXPLAINED}`);
    } else if (r.status === "failed") {
      if (before !== undefined) different[r.rule] = before;
      if (declaration.untranslated[r.rule] !== undefined) untranslated[r.rule] = declaration.untranslated[r.rule]!;
      else if (!gating && before === undefined) untranslated[r.rule] = `failed: ${r.message ?? ""}`;
    } else if (r.status === "different") {
      const counts: Record<string, Counts> = {};
      for (const [corpus, c] of Object.entries(before?.counts ?? {})) if (!ran.includes(corpus)) counts[corpus] = c;
      Object.assign(counts, countsOf(r));
      different[r.rule] = { reason: before?.reason ?? (gating ? "" : NOT_EXPLAINED), right: before?.right ?? "unknown", counts: sortedRecord(counts) };
    }
  }
  const out: Declaration = { format: DECLARATION_FORMAT, pack: declaration.pack, equivalent: declaration.equivalent, untranslated, different };
  if (Object.keys(incomplete).length > 0) out.incomplete = incomplete;
  return out;
}

// ---------------------------------------------------------------- examples

/** One example file of a rule (pack.json `rules[].examples.caught|passes`). */
export interface ExampleFile {
  rule: string;
  kind: string;
  path: string;
  text: string;
}

/** A pack.json's example files, and the examples that are not files (no relative path), with why. */
export function exampleFiles(packJson: { rules?: { id: string; examples?: Record<string, { path?: string; text?: string } | undefined> }[] }): { files: ExampleFile[]; skipped: string[] } {
  const files: ExampleFile[] = [];
  const skipped: string[] = [];
  for (const rule of packJson.rules ?? []) {
    for (const [kind, example] of Object.entries(rule.examples ?? {})) {
      if (example === undefined) continue;
      const path = example.path;
      if (typeof example.text !== "string") skipped.push(`${rule.id} ${kind}: no text`);
      else if (path === undefined || path === "") skipped.push(`${rule.id} ${kind}: no path (not a file's example)`);
      else if (path.startsWith("/") || path.includes("\\") || path.split("/").some((part) => part === ".." || part === "." || part === "")) skipped.push(`${rule.id} ${kind}: path ${path} is not a plain relative path`);
      else files.push({ rule: rule.id, kind, path, text: example.text });
    }
  }
  return { files, skipped };
}

/**
 * The example files laid out as few corpora as can hold them: in order, each into the first corpus that has no other
 * text at its path, nor a file where a directory of it would go (or the reverse). Two examples of one text at one path
 * share it.
 */
export function layOutExamples(files: readonly ExampleFile[]): Map<string, string>[] {
  const bins: Map<string, string>[] = [];
  const fits = (bin: Map<string, string>, path: string, text: string): boolean => {
    const there = bin.get(path);
    if (there !== undefined) return there === text;
    for (const other of bin.keys()) if (other.startsWith(`${path}/`) || path.startsWith(`${other}/`)) return false;
    return true;
  };
  for (const f of files) {
    let bin = bins.find((b) => fits(b, f.path, f.text));
    if (bin === undefined) bins.push((bin = new Map()));
    bin.set(f.path, f.text);
  }
  return bins;
}

// ---------------------------------------------------------------- reporting

/** One pack's run, as --out's JSON holds it. */
export interface PackReport {
  pack: string;
  equivalent: boolean;
  /** Each corpus compared: its name, directory and file count. */
  corpora: { name: string; dir: string; files: number }[];
  /** Corpora the run could not compare, with why. */
  incomplete: Record<string, string>;
  /** Example files that are not files (pack.json), and translations without an original rule. */
  notes: string[];
  summary: Record<Status, number>;
  rules: RuleResult[];
  /** The gate's problems (a pack that does not gate lists them too, without failing). */
  problems: string[];
  /** Whether the pack fails the run: it gates and has problems. */
  failed: boolean;
}

/** A pack's text summary: its counts, each rule that is not the same, and the gate's problems. */
export function renderPack(report: PackReport, detail = true): string {
  const s = report.summary;
  const lines = [`${report.pack}: ${s.same} same, ${s.different} different, ${s.untranslated} untranslated, ${s.failed} failed (${report.rules.length} rules; corpora ${report.corpora.map((c) => `${c.name} ${c.files} files`).join(", ") || "none"})`];
  for (const [corpus, why] of Object.entries(report.incomplete)) lines.push(`  not compared on ${corpus}: ${why}`);
  if (detail) {
    for (const r of report.rules) {
      if (r.status === "same") continue;
      if (r.status !== "different") {
        lines.push(`  ${r.status} ${r.rule}: ${(r.message ?? "").slice(0, 400)}`);
        continue;
      }
      lines.push(`  different ${r.rule}: ${Object.entries(countsOf(r)).map(([corpus, c]) => `${corpus} ${countsText(c)}`).join("; ")}`);
      for (const [corpus, c] of Object.entries(r.corpora)) {
        for (const key of c.samples.originalOnly) lines.push(`    ${corpus} original only: ${key}`);
        for (const key of c.samples.translationOnly) lines.push(`    ${corpus} translation only: ${key}`);
        if (c.originalFailures !== undefined) lines.push(`    ${corpus}: the original failed on ${c.originalFailures.count} file(s); the first: ${c.originalFailures.first.slice(0, 200)}`);
      }
    }
  }
  const verdict = report.equivalent ? (report.failed ? "FAILS its declaration" : "matches its declaration") : "does not gate (equivalent: false)";
  lines.push(`  ${report.pack} ${verdict}${report.problems.length === 0 ? "" : `: ${report.problems.length} problem(s)`}`);
  for (const problem of report.problems) lines.push(`    ${problem}`);
  return lines.join("\n");
}
