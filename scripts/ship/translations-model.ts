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

/** The corpus of a pack's tool-call rules' examples, each a coding agent's tool call (exampleToolCalls). */
export const TOOL_CALLS_CORPUS = "tool-calls";

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
   * reports them and carries on, so the rule is compared still, and a difference on such a file may be theirs. `other`
   * counts those that are not a file's syntax errors (isParseFailure), with the first of them: the gate does not pass a
   * same rule with any, as its findings on those files were not compared.
   */
  originalFailures?: { count: number; first: string; other?: { count: number; first: string } };
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

/** Keys both sides report, and each side's own as people read them. */
interface Tally {
  both: number;
  originalOnly: string[];
  translationOnly: string[];
}

/** One key both sides report: once in both when a side gives no column, else each column both give, and each side's own. */
function tallyColumns(tally: Tally, key: string, oc: ReadonlySet<number>, tc: ReadonlySet<number>): void {
  if (oc.size === 0 || tc.size === 0) {
    tally.both += 1;
    return;
  }
  tally.both += [...oc].filter((c) => tc.has(c)).length;
  tally.originalOnly.push(...[...oc].filter((c) => !tc.has(c)).map((c) => shown(key, c)));
  tally.translationOnly.push(...[...tc].filter((c) => !oc.has(c)).map((c) => shown(key, c)));
}

/**
 * Two sides' findings of one rule compared as sets of (rule, path, line, code), and by column too at a key where both
 * sides give columns: a column only one side reports is that side's own. Messages are not compared.
 */
export function compareFindings(original: readonly Finding[], translation: readonly Finding[]): CorpusComparison {
  const o = byKey(original);
  const t = byKey(translation);
  const tally: Tally = { both: 0, originalOnly: [], translationOnly: [] };
  for (const [key, oc] of o) {
    const tc = t.get(key);
    if (tc === undefined) tally.originalOnly.push(shown(key));
    else tallyColumns(tally, key, oc, tc);
  }
  for (const key of t.keys()) if (!o.has(key)) tally.translationOnly.push(shown(key));
  tally.originalOnly.sort();
  tally.translationOnly.sort();
  return {
    both: tally.both,
    originalOnly: tally.originalOnly.length,
    translationOnly: tally.translationOnly.length,
    samples: { originalOnly: tally.originalOnly.slice(0, SAMPLES), translationOnly: tally.translationOnly.slice(0, SAMPLES) },
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

/**
 * The heads of an original's failures on a file (`code: message`) that say only that the file has syntax errors: rules
 * ran around them (`source-partial`), a rule that needs the whole file skipped it, or the parser did not read it
 * (src/engine/audit/audit.ts `isSourceGap`, binary files aside).
 */
const PARSE_FAILURE_HEADS = ["source-partial: ", "source-incomplete: file has syntax errors", "source-incomplete: file did not parse: "];

/** Whether an original's failure on a file (`code: message`) says only that the file has syntax errors. */
export function isParseFailure(failure: string): boolean {
  return PARSE_FAILURE_HEADS.some((head) => failure.startsWith(head));
}

/** A corpus's original failures as a comparison keeps them: how many, the first, and those that are not parse failures. */
function originalFailuresOf(failures: readonly string[] | undefined): CorpusComparison["originalFailures"] {
  if (failures === undefined || failures.length === 0) return undefined;
  const other = failures.filter((failure) => !isParseFailure(failure));
  return { count: failures.length, first: failures[0]!, ...(other.length === 0 ? {} : { other: { count: other.length, first: other[0]! } }) };
}

/** The failures that leave a rule without findings to compare, each said once after the corpora it was met on. */
function corpusFailures(corpora: RuleInput["corpora"]): string | undefined {
  const failures = new Map<string, string[]>();
  for (const [corpus, run] of Object.entries(corpora)) if ("failure" in run) failures.set(run.failure, [...(failures.get(run.failure) ?? []), corpus]);
  return failures.size === 0 ? undefined : [...failures].map(([why, where]) => `${where.join(", ")}: ${why}`).join("; ");
}

/** Each corpus's comparison of a rule both sides ran, with the original's failures on its files. */
function comparedCorpora(corpora: RuleInput["corpora"]): Record<string, CorpusComparison> {
  const out: Record<string, CorpusComparison> = {};
  for (const [corpus, run] of Object.entries(corpora)) {
    if ("failure" in run) continue;
    const c = compareFindings(run.original, run.translation);
    const originalFailures = originalFailuresOf(run.originalFailures);
    out[corpus] = originalFailures === undefined ? c : { ...c, originalFailures };
  }
  return out;
}

/** A rule's outcome: untranslated, failed (refused, or a side failed on a corpus), different, or same. */
export function ruleResult(input: RuleInput): RuleResult {
  if (input.translation === undefined) return { rule: input.rule, status: "untranslated", message: "no .rerail translation", corpora: {} };
  if (input.refused !== undefined) return { rule: input.rule, status: "failed", message: input.refused, corpora: {} };
  const failure = corpusFailures(input.corpora);
  if (failure !== undefined) return { rule: input.rule, status: "failed", message: failure, corpora: {} };
  const corpora = comparedCorpora(input.corpora);
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

/** What makes an equivalence.json not a declaration: the run cannot read what its pack expects. */
export class DeclarationError extends Error {}

/** Says why a declaration's text is not one, and does not return. */
type Fail = (why: string) => never;

/** Whether a JSON value is an object (not null, not an array). */
function isObject(value: unknown): value is Record<string, unknown> {
  return typeof value === "object" && value !== null && !Array.isArray(value);
}

/** The declaration's JSON object, its format, pack and `equivalent` checked. */
function declarationObject(text: string, pack: string, fail: Fail): Record<string, unknown> {
  let d: unknown;
  try {
    d = JSON.parse(text);
  } catch (error) {
    return fail(`not JSON: ${(error as Error).message}`);
  }
  if (!isObject(d)) return fail("not a JSON object");
  if (d.format !== DECLARATION_FORMAT) fail(`format is ${JSON.stringify(d.format)}, not ${DECLARATION_FORMAT}`);
  if (d.pack !== pack) fail(`names pack ${JSON.stringify(d.pack)}`);
  if (typeof d.equivalent !== "boolean") fail("equivalent is not true or false");
  return d;
}

/** A field of the declaration that is an object. */
function objectField(d: Record<string, unknown>, name: string, fail: Fail): Record<string, unknown> {
  const field = d[name];
  return isObject(field) ? field : fail(`${name} is not an object`);
}

/** A field of the declaration that maps names to texts (`untranslated`, `incomplete`). */
function textsField(d: Record<string, unknown>, name: string, fail: Fail): Record<string, string> {
  const field = objectField(d, name, fail);
  for (const [key, why] of Object.entries(field)) if (typeof why !== "string") fail(`${name}.${key} is not a text`);
  return field as Record<string, string>;
}

/** A `different` entry's counts, per corpus. */
function entryCounts(rule: string, field: Record<string, unknown>, fail: Fail): Record<string, Counts> {
  const counts: Record<string, Counts> = {};
  for (const [corpus, c] of Object.entries(field)) {
    const n = c as Partial<Counts> | null;
    if (n === null || !Number.isInteger(n.originalOnly) || !Number.isInteger(n.translationOnly)) fail(`different.${rule}.counts.${corpus} is not { originalOnly, translationOnly }`);
    counts[corpus] = { originalOnly: n!.originalOnly!, translationOnly: n!.translationOnly! };
  }
  return counts;
}

/** One `different` entry: its reason, the side that is right, and its counts. */
function differenceEntry(rule: string, entry: unknown, fail: Fail): Difference {
  if (typeof entry !== "object" || entry === null) fail(`different.${rule} is not an object`);
  const e = entry as Record<string, unknown>;
  if (typeof e.reason !== "string") fail(`different.${rule}.reason is not a text`);
  if (!RIGHTS.includes(e.right as Right)) fail(`different.${rule}.right is not one of ${RIGHTS.join(", ")}`);
  if (!isObject(e.counts)) return fail(`different.${rule}.counts is not an object`);
  return { reason: e.reason as string, right: e.right as Right, counts: entryCounts(rule, e.counts, fail) };
}

/** A declaration read from its JSON text; a DeclarationError says why the text is not one. */
export function parseDeclaration(text: string, pack: string): Declaration {
  const fail: Fail = (why) => {
    throw new DeclarationError(`equivalence.json of ${pack}: ${why}`);
  };
  const d = declarationObject(text, pack, fail);
  const untranslated = textsField(d, "untranslated", fail);
  const different: Record<string, Difference> = {};
  for (const [rule, entry] of Object.entries(objectField(d, "different", fail))) different[rule] = differenceEntry(rule, entry, fail);
  const declaration: Declaration = { format: DECLARATION_FORMAT, pack, equivalent: d.equivalent as boolean, untranslated, different };
  if (d.incomplete !== undefined) declaration.incomplete = textsField(d, "incomplete", fail);
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

/** An untranslated rule's disagreements: not listed in `untranslated` (or, gating, listed without why), or still listed in `different`. */
function untranslatedProblems(declaration: Declaration, r: RuleResult): string[] {
  const listed = declaration.untranslated[r.rule];
  const problems: string[] = [];
  if (listed === undefined) problems.push(`${r.rule}: has no translation and is not listed in untranslated`);
  else if (declaration.equivalent && listed.trim() === "") problems.push(`${r.rule}: listed in untranslated without the engine's refusal or the feature it lacks`);
  if (declaration.different[r.rule] !== undefined) problems.push(`${r.rule}: has no translation; remove it from different`);
  return problems;
}

/** A same rule's disagreement: still listed in `different`. */
function sameProblems(declaration: Declaration, r: RuleResult): string[] {
  return declaration.different[r.rule] === undefined ? [] : [`${r.rule}: reports the same findings as its original now; remove it from different`];
}

/**
 * A translated rule's original, in a gating pack, failed on files of a corpus for another reason than their syntax
 * errors, so the rule's findings there were not compared: whether its findings are the same or listed as different.
 */
function originalFailureProblems(declaration: Declaration, r: RuleResult): string[] {
  if (!declaration.equivalent) return [];
  const problems: string[] = [];
  for (const [corpus, c] of Object.entries(r.corpora)) {
    const other = c.originalFailures?.other;
    if (other !== undefined) problems.push(`${r.rule}: on ${corpus}, the original failed on ${other.count} file(s) for another reason than syntax errors; the first: ${other.first.slice(0, 200)}`);
  }
  return problems;
}

/** A listed different rule's counts that changed on a corpus the run compared. */
function changedCounts(rule: string, listed: Difference, counts: Record<string, Counts>, ran: readonly string[]): string[] {
  const problems: string[] = [];
  for (const corpus of ran) {
    const declared = listed.counts[corpus] ?? NONE;
    const now = counts[corpus] ?? NONE;
    if (declared.originalOnly !== now.originalOnly || declared.translationOnly !== now.translationOnly) problems.push(`${rule}: on ${corpus}, declared ${countsText(declared)}, now ${countsText(now)}`);
  }
  return problems;
}

/** A gating pack's different entry without its reason or its side, or with counts on a corpus the run did not compare. */
function unexplained(rule: string, listed: Difference, ran: readonly string[]): string[] {
  const problems: string[] = [];
  if (listed.reason.trim() === "") problems.push(`${rule}: listed in different without a reason`);
  if (listed.right === "unknown") problems.push(`${rule}: listed in different without saying which side is right (original, translation or neither)`);
  for (const corpus of Object.keys(listed.counts)) if (!ran.includes(corpus)) problems.push(`${rule}: listed in different with counts on ${corpus}, which the run did not compare`);
  return problems;
}

/** A different rule's disagreements: not listed in `different`, listed without its explanation, or its counts changed. */
function differentProblems(declaration: Declaration, r: RuleResult, ran: readonly string[]): string[] {
  const listed = declaration.different[r.rule];
  const counts = countsOf(r);
  if (listed === undefined) {
    const where = Object.entries(counts).map(([corpus, c]) => `${corpus}: ${countsText(c)}`).join("; ");
    return [`${r.rule}: reports different findings and is not listed in different (${where})`];
  }
  return [...(declaration.equivalent ? unexplained(r.rule, listed, ran) : []), ...changedCounts(r.rule, listed, counts, ran)];
}

/**
 * A translated rule's disagreements: still listed in `untranslated`, then those of a same or a different rule, then
 * its original's failures on files.
 */
function translatedProblems(declaration: Declaration, r: RuleResult, ran: readonly string[]): string[] {
  const problems = declaration.untranslated[r.rule] === undefined ? [] : [`${r.rule}: is translated now; remove it from untranslated`];
  problems.push(...(r.status === "same" ? sameProblems(declaration, r) : differentProblems(declaration, r, ran)));
  problems.push(...originalFailureProblems(declaration, r));
  return problems;
}

/** One rule's disagreements with the declaration. */
function ruleProblems(declaration: Declaration, r: RuleResult, ran: readonly string[]): string[] {
  switch (r.status) {
    case "failed":
      return [`${r.rule}: failed: ${r.message ?? ""}`];
    case "untranslated":
      return untranslatedProblems(declaration, r);
    default:
      return translatedProblems(declaration, r, ran);
  }
}

/** A gating pack's run that compared nothing, or whose original failed in ways attributed to no rule, per corpus. */
function runProblems(ran: readonly string[], unattributed: Readonly<Record<string, readonly string[]>>): string[] {
  const problems = ran.length === 0 ? ["the run compared no corpus"] : [];
  for (const [corpus, failures] of Object.entries(unattributed)) if (failures.length > 0) problems.push(`on ${corpus}, the original's run had ${failures.length} failure(s) of no rule; the first: ${failures[0]!.slice(0, 200)}`);
  return problems;
}

/**
 * Every way a run disagrees with the pack's declaration: a translated rule that differs and is not listed in
 * `different`, a listed rule whose counts changed on a corpus the run compared or that is the same now, an entry without
 * its reason, a rule without a translation not listed in `untranslated` (or listed and translated now, or listed in
 * `different`), a rule that failed, an entry naming no rule of the pack. `ran` names the corpora the run compared:
 * counts of others are not checked, except that a pack that gates may list counts only on those (its run compares all
 * its corpora, or fails). A pack that gates fails too on a run that compared no corpus, on a translated rule (same or
 * different) whose original failed on files of a corpus for another reason than their syntax errors (isParseFailure),
 * and on the original's failures of no rule (`unattributed`, per corpus: `code: message`). Only a pack declaring
 * `equivalent: true` fails on them (gates); the rest report them.
 */
export function gateProblems(declaration: Declaration, results: readonly RuleResult[], ran: readonly string[], pack: readonly string[], unattributed: Readonly<Record<string, readonly string[]>> = {}): string[] {
  const problems = results.flatMap((r) => ruleProblems(declaration, r, ran));
  if (declaration.equivalent) problems.push(...runProblems(ran, unattributed));
  const known = new Set(pack);
  problems.push(...Object.keys(declaration.different).filter((rule) => !known.has(rule)).map((rule) => `${rule}: listed in different, but the pack has no such rule`));
  problems.push(...Object.keys(declaration.untranslated).filter((rule) => !known.has(rule)).map((rule) => `${rule}: listed in untranslated, but the pack has no such rule`));
  return problems;
}

/** A failed rule's entries kept as they were; in a pack that does not gate, one without any is listed untranslated with its failure. */
function keepFailed(declaration: Declaration, r: RuleResult, out: Pick<Declaration, "untranslated" | "different">): void {
  const before = declaration.different[r.rule];
  const listed = declaration.untranslated[r.rule];
  if (before !== undefined) out.different[r.rule] = before;
  if (listed !== undefined) out.untranslated[r.rule] = listed;
  else if (!declaration.equivalent && before === undefined) out.untranslated[r.rule] = `failed: ${r.message ?? ""}`;
}

/** A different rule's entry: the one there with the run's counts, keeping those of corpora the run did not compare. */
function differentEntry(declaration: Declaration, r: RuleResult, ran: readonly string[]): Difference {
  const before = declaration.different[r.rule];
  const counts: Record<string, Counts> = {};
  for (const [corpus, c] of Object.entries(before?.counts ?? {})) if (!ran.includes(corpus)) counts[corpus] = c;
  Object.assign(counts, countsOf(r));
  return { reason: before?.reason ?? (declaration.equivalent ? "" : NOT_EXPLAINED), right: before?.right ?? "unknown", counts: sortedRecord(counts) };
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
  const out: Declaration = { format: DECLARATION_FORMAT, pack: declaration.pack, equivalent: declaration.equivalent, untranslated: {}, different: {} };
  for (const r of results) {
    switch (r.status) {
      case "untranslated":
        out.untranslated[r.rule] = declaration.untranslated[r.rule] ?? (declaration.equivalent ? "" : `${r.message ?? "no .rerail translation"}; ${NOT_EXPLAINED}`);
        break;
      case "failed":
        keepFailed(declaration, r, out);
        break;
      case "different":
        out.different[r.rule] = differentEntry(declaration, r, ran);
        break;
    }
  }
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

/** Whether an example's path is a plain relative path: not absolute, no backslash, no empty, `.` or `..` part. */
function isPlainRelativePath(path: string): boolean {
  return !(path.startsWith("/") || path.includes("\\") || path.split("/").some((part) => part === ".." || part === "." || part === ""));
}

/** A rule's example as a file, or why it is not one. */
function exampleFile(rule: string, kind: string, example: { path?: string; text?: string }): ExampleFile | string {
  const path = example.path;
  if (typeof example.text !== "string") return "no text";
  if (path === undefined || path === "") return "no path (not a file's example)";
  if (!isPlainRelativePath(path)) return `path ${path} is not a plain relative path`;
  return { rule, kind, path, text: example.text };
}

/** A pack.json's example files, and the examples that are not files (no relative path), with why. */
export function exampleFiles(packJson: { rules?: { id: string; examples?: Record<string, { path?: string; text?: string } | undefined> }[] }): { files: ExampleFile[]; skipped: string[] } {
  const files: ExampleFile[] = [];
  const skipped: string[] = [];
  for (const rule of packJson.rules ?? []) {
    for (const [kind, example] of Object.entries(rule.examples ?? {})) {
      if (example === undefined) continue;
      const file = exampleFile(rule.id, kind, example);
      if (typeof file === "string") skipped.push(`${rule.id} ${kind}: ${file}`);
      else files.push(file);
    }
  }
  return { files, skipped };
}

/** One tool call, as `rerail-engine run-policies --tool-calls` reads it. */
export interface ExampleToolCall {
  /** Where it comes from: `<rule> <example kind>`, or `<rule> <file>:<line>` for a test's call. */
  id: string;
  tool: string;
  input: Record<string, unknown>;
}

/**
 * The examples of a pack's tool-call rules (`sources`: each one's `sample.source`) as tool calls: an example's text is
 * the command the coding agent runs, through the tool its source names (`tool:*` tried as Bash, as
 * conformance/pack-validation.ts tries it). An example without text is not a call, and is skipped with why.
 */
export function exampleToolCalls(packJson: { rules?: { id: string; examples?: Record<string, { text?: string } | undefined> }[] }, sources: ReadonlyMap<string, string>): { calls: ExampleToolCall[]; skipped: string[] } {
  const out: { calls: ExampleToolCall[]; skipped: string[] } = { calls: [], skipped: [] };
  for (const rule of packJson.rules ?? []) {
    const source = sources.get(rule.id);
    if (source !== undefined) ruleToolCalls(rule, source === "tool:*" ? "Bash" : source.slice("tool:".length), out);
  }
  return out;
}

/** One tool-call rule's examples as calls through `tool`, into `out`: each example with text, the others skipped. */
function ruleToolCalls(rule: { id: string; examples?: Record<string, { text?: string } | undefined> }, tool: string, out: { calls: ExampleToolCall[]; skipped: string[] }): void {
  for (const [kind, example] of Object.entries(rule.examples ?? {})) {
    if (example === undefined) continue;
    if (typeof example.text !== "string") out.skipped.push(`${rule.id} ${kind}: no text`);
    else out.calls.push({ id: `${rule.id} ${kind}`, tool, input: { command: example.text } });
  }
}

/** A test's call in a `.rerail` file: `.tool_call "<tool>" `<input as JSON>``. */
const TOOL_CALL_LINE = /^\s*\.tool_call\s+"([^"]+)"\s+`(.*)`\s*$/;

/**
 * The calls a tool-call rule's translation tests with (its `.tool_call` lines, in `file`, named by its line), so the
 * original is run on each of them too: its tests check the translation against what its author expects, this checks it
 * against the original. An input that is not a JSON object is skipped with why.
 */
export function testToolCalls(rule: string, file: string, text: string): { calls: ExampleToolCall[]; skipped: string[] } {
  const out: { calls: ExampleToolCall[]; skipped: string[] } = { calls: [], skipped: [] };
  text.split("\n").forEach((line, i) => {
    const m = TOOL_CALL_LINE.exec(line);
    if (m !== null) testToolCall(`${rule} ${file}:${i + 1}`, m[1]!, m[2]!, out);
  });
  return out;
}

/** One test call, its input read as JSON, into `out`. */
function testToolCall(id: string, tool: string, raw: string, out: { calls: ExampleToolCall[]; skipped: string[] }): void {
  let input: unknown;
  try {
    input = JSON.parse(raw);
  } catch {
    input = undefined;
  }
  if (isObject(input)) out.calls.push({ id, tool, input });
  else out.skipped.push(`${id}: its input is not a JSON object`);
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

/** A different rule's samples on one corpus, and the original's failures on its files. */
function corpusLines(corpus: string, c: CorpusComparison): string[] {
  const lines = [...c.samples.originalOnly.map((key) => `    ${corpus} original only: ${key}`), ...c.samples.translationOnly.map((key) => `    ${corpus} translation only: ${key}`)];
  if (c.originalFailures !== undefined) lines.push(`    ${corpus}: the original failed on ${c.originalFailures.count} file(s); the first: ${c.originalFailures.first.slice(0, 200)}`);
  return lines;
}

/** A rule's lines in a pack's summary: none when it is the same, its message when it failed or is untranslated, else its counts and samples. */
function ruleLines(r: RuleResult): string[] {
  if (r.status === "same") return [];
  if (r.status !== "different") return [`  ${r.status} ${r.rule}: ${(r.message ?? "").slice(0, 400)}`];
  return [`  different ${r.rule}: ${Object.entries(countsOf(r)).map(([corpus, c]) => `${corpus} ${countsText(c)}`).join("; ")}`, ...Object.entries(r.corpora).flatMap(([corpus, c]) => corpusLines(corpus, c))];
}

/** Whether the pack gates, and matches its declaration or fails it, with how many problems it has. */
function verdictLine(report: PackReport): string {
  const verdict = report.equivalent ? (report.failed ? "FAILS its declaration" : "matches its declaration") : "does not gate (equivalent: false)";
  return `  ${report.pack} ${verdict}${report.problems.length === 0 ? "" : `: ${report.problems.length} problem(s)`}`;
}

/** A pack's text summary: its counts, each rule that is not the same, and the gate's problems. */
export function renderPack(report: PackReport, detail = true): string {
  const s = report.summary;
  const corpora = report.corpora.map((c) => `${c.name} ${c.files} files`).join(", ") || "none";
  const lines = [`${report.pack}: ${s.same} same, ${s.different} different, ${s.untranslated} untranslated, ${s.failed} failed (${report.rules.length} rules; corpora ${corpora})`];
  for (const [corpus, why] of Object.entries(report.incomplete)) lines.push(`  not compared on ${corpus}: ${why}`);
  if (detail) lines.push(...report.rules.flatMap((r) => ruleLines(r)));
  lines.push(verdictLine(report));
  for (const problem of report.problems) lines.push(`    ${problem}`);
  return lines.join("\n");
}
