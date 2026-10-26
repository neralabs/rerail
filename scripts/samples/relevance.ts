/**
 * Which clean files a sample set keeps (D2). A file with a labelled violation for any rule is kept with all its labels.
 * A file clean for every rule it is labelled for is kept for a rule only when it is a near miss for that rule: it holds
 * the construct the rule inspects, yet upstream reported nothing there. Of a rule's near misses the smallest few are
 * kept (by size, then sha256); a rule with none keeps its one smallest clean file.
 *
 * "Holds the construct the rule inspects" is decided by the engine, not guessed from text. The engine's own anchor plan
 * (the root node kinds a compiled query selects before any condition runs, native `matcher/anchors.rs`) is not exposed
 * to JavaScript, so the builder asks the engine the same question through a relaxed query: the rule's document with
 * each AST clause cut down to the node it anchors on (`kind`, `pattern`, the `matches` utilities they name, and the
 * `any`/`all`/`$option` combinations of those), dropping every condition on the node (`has`, `inside`, `not`, `regex`,
 * `resolves`, `when`, `constraints`, `file`, ...). A file where the relaxed query reports a node has a candidate node
 * for the rule. A clause whose root names no node of its own anchors on the node its first relation names (`inside`,
 * `has`, `follows`, `precedes`), which the file must hold for the clause to match. A text (`regex`) clause, or a
 * clause that anchors on nothing at all, selects no candidate: the rule then keeps only its smallest clean file.
 */
import { readFileSync } from "node:fs";

import { evaluateWorkspace, type SourceFile } from "../../conformance/languages/runner.js";
import { capabilitiesJson } from "../../conformance/engine.js";
import { languageFor, loadNativeBinding } from "../../src/engine/index.js";
import { byText, hasPositive, sampleId, sampleKey, type Sample } from "./samples.js";

/** The most near misses a rule keeps (D2). */
export const NEAR_MISS_CAP = 5;

type Json = Record<string, unknown>;

const isObject = (value: unknown): value is Json => typeof value === "object" && value !== null && !Array.isArray(value);

/** The relations whose target a clause anchoring on nothing of its own must find in the file, in the order tried. */
const RELATIONS = ["inside", "has", "follows", "precedes"] as const;

/**
 * The part of an AST rule that picks its root node: `kind`, `pattern`, a `matches` utility's own (inlined), and
 * `any`/`all`/`$option` of those; undefined when it picks any node (no such key, or an `any` with an alternative that
 * picks any node). `seen` guards a recursive utility.
 */
export function anchorOf(rule: unknown, utils: Readonly<Json>, seen: ReadonlySet<string> = new Set()): Json | undefined {
  if (!isObject(rule)) return undefined;
  const parts: Json[] = [];
  if (typeof rule.kind === "string") parts.push({ kind: rule.kind });
  if (rule.pattern !== undefined) parts.push({ pattern: rule.pattern });
  if (typeof rule.matches === "string" && !seen.has(rule.matches) && utils[rule.matches] !== undefined) {
    const inlined = anchorOf(utils[rule.matches], utils, new Set([...seen, rule.matches]));
    if (inlined !== undefined) parts.push(inlined);
  }
  if (Array.isArray(rule.all)) for (const member of rule.all) {
    const anchor = anchorOf(member, utils, seen);
    if (anchor !== undefined) parts.push(anchor);
  }
  const alternatives = alternativesOf(rule);
  if (alternatives !== undefined) {
    // An alternative that anchors on no node of its own lets `any` match any node: it narrows nothing then.
    const anchors = alternatives.map((alternative) => anchorOf(alternative, utils, seen));
    if (anchors.every((anchor) => anchor !== undefined)) parts.push(anchors.length === 1 ? anchors[0]! : { any: anchors });
  }
  if (parts.length === 0) return undefined;
  return parts.length === 1 ? parts[0] : { all: parts };
}

/** The alternatives of an `any`, or of an `$option`'s cases and default; undefined for a rule with neither. */
function alternativesOf(rule: Json): unknown[] | undefined {
  if (Array.isArray(rule.any) && rule.any.length > 0) return rule.any;
  if (rule.$option !== undefined && isObject(rule.cases)) return [...Object.values(rule.cases), ...(rule.default === undefined ? [] : [rule.default])];
  return undefined;
}

/**
 * For a rule that anchors on no node of its own, a node the file must still hold for it to match: the candidate of
 * the first relation it or a conjunct names, or of each of its alternatives.
 */
function relationAnchor(rule: unknown, utils: Readonly<Json>, seen: ReadonlySet<string> = new Set()): Json | undefined {
  if (!isObject(rule)) return undefined;
  for (const relation of RELATIONS) {
    const anchor = isObject(rule[relation]) ? (anchorOf(rule[relation], utils, seen) ?? relationAnchor(rule[relation], utils, seen)) : undefined;
    if (anchor !== undefined) return anchor;
  }
  if (Array.isArray(rule.all)) for (const member of rule.all) {
    const anchor = relationAnchor(member, utils, seen);
    if (anchor !== undefined) return anchor;
  }
  const alternatives = alternativesOf(rule);
  if (alternatives !== undefined) {
    const anchors = alternatives.map((alternative) => anchorOf(alternative, utils, seen) ?? relationAnchor(alternative, utils, seen));
    if (anchors.every((anchor) => anchor !== undefined)) return anchors.length === 1 ? anchors[0] : { any: anchors };
  }
  if (typeof rule.matches === "string" && !seen.has(rule.matches)) return relationAnchor(utils[rule.matches], utils, new Set([...seen, rule.matches]));
  return undefined;
}

/** A rule's anchor, or for one that anchors on no node of its own, a node its relations need (relationAnchor). */
export function candidateRule(rule: unknown, utils: Readonly<Json>): Json | undefined {
  return anchorOf(rule, utils) ?? relationAnchor(rule, utils);
}

const PROBE_VIOLATION = { anchor: "witness.span", code: "NEAR-MISS-PROBE", message: "a node the rule inspects" };

/**
 * The relaxed query of a rule document: the same document (scope, options) whose clauses are its AST clauses each cut
 * to its candidate rule, reporting every candidate node; undefined when no clause has one.
 */
export function relaxedDocument(document: Json): Json | undefined {
  const clauses = Array.isArray(document.clauses) ? document.clauses.filter(isObject) : [];
  const shared: Json = {};
  for (const clause of clauses) if (isObject(clause.define) && isObject(clause.define.utils)) Object.assign(shared, clause.define.utils);
  const relaxed = clauses.flatMap((clause, index) => {
    const ast = isObject(clause.forbid) ? clause.forbid.ast : isObject(clause.require) ? clause.require.ast : undefined;
    if (!isObject(ast)) return [];
    // An AST clause chosen by a document option (`$option`) is any of its cases: each variant is a candidate query.
    const variants = ast.$option !== undefined && isObject(ast.cases) ? [...Object.values(ast.cases), ast.default].filter(isObject) : [ast];
    return variants.flatMap((variant, position) => {
      if (typeof variant.language !== "string") return [];
      const rule = candidateRule(variant.rule, { ...shared, ...(isObject(variant.utils) ? variant.utils : {}) });
      if (rule === undefined) return [];
      const kept: Json = { language: variant.language, rule };
      if (variant.languageOptions !== undefined) kept.languageOptions = variant.languageOptions;
      if (variant.embedded !== undefined) kept.embedded = variant.embedded;
      return [{ id: `candidate-${index}-${position}`, forbid: { ast: kept }, violation: PROBE_VIOLATION }];
    });
  });
  return relaxed.length === 0 ? undefined : { ...document, clauses: relaxed };
}

/**
 * The samples among `samples` where the rule's relaxed query (relaxedDocument) selects a node, by sampleKey; each is
 * evaluated under its archive path, as the conformance check reads it. Empty when the rule has no relaxed query.
 */
export async function nearMissCandidates(documentPath: string, samples: readonly Sample[]): Promise<{ selected: Set<string>; probe: string }> {
  const relaxed = relaxedDocument(JSON.parse(readFileSync(documentPath, "utf8")) as Json);
  if (relaxed === undefined) return { selected: new Set(), probe: "none: no clause anchors on a node" };
  let compiled;
  try {
    compiled = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(relaxed), "json", capabilitiesJson());
  } catch (error) {
    return { selected: new Set(), probe: `none: the relaxed query does not compile (${error instanceof Error ? error.message.split("\n")[0] : String(error)})` };
  }
  const byPath = new Map<string, string>();
  const files: SourceFile[] = [];
  for (const sample of samples) {
    const path = `${sampleId(sample.short, sample.source.path, sample.source.sha256)}/${sample.source.path}`;
    const language = languageFor(path);
    if (language === undefined) continue;
    byPath.set(path, sampleKey(sample));
    files.push({ path, language, source: sample.bytes.toString("utf8") });
  }
  const evaluation = await evaluateWorkspace([compiled], files);
  const selected = new Set<string>();
  for (const violation of evaluation.violations) {
    const key = violation.path === undefined ? undefined : byPath.get(violation.path);
    if (key !== undefined) selected.add(key);
  }
  return { selected, probe: "relaxed query" };
}

/** Why a sample carries, or lost, its clean label for a rule. */
export type CleanOutcome =
  | { kept: true; reason: "on a file with a labelled violation" | "a chosen clean sample" | "near miss" | "smallest clean file (no near miss)" }
  | { kept: false; reason: "not relevant" | "over the cap" };

export interface Selection {
  /** The kept samples, each clean-only one carrying only the clean keys of the rules that kept it. */
  samples: Sample[];
  /** sampleKey -> pack rule id -> outcome of its clean label. */
  clean: Map<string, Map<string, CleanOutcome>>;
}

const bySize = (a: Sample, b: Sample): number => a.bytes.length - b.bytes.length || byText(a.source.sha256, b.source.sha256) || byText(sampleKey(a), sampleKey(b));

/**
 * D2 over deduplicated samples. `nearMisses` holds, per pack rule id, the sampleKeys of clean-only samples whose
 * relaxed query selects a node. A pinned sample (the ESLint test set, a licence replacement) is always kept whole.
 */
export function selectRelevant(samples: readonly Sample[], nearMisses: ReadonlyMap<string, ReadonlySet<string>>): Selection {
  const clean = new Map<string, Map<string, CleanOutcome>>();
  const record = (sample: Sample, rule: string, outcome: CleanOutcome): void => {
    const key = sampleKey(sample);
    clean.set(key, (clean.get(key) ?? new Map()).set(rule, outcome));
  };
  const whole = samples.filter((sample) => sample.pinned || hasPositive(sample));
  for (const sample of whole) {
    const reason = hasPositive(sample) ? "on a file with a labelled violation" : "a chosen clean sample";
    for (const [rule, labels] of sample.expected) if (labels.length === 0) record(sample, rule, { kept: true, reason });
  }
  const cleanOnly = samples.filter((sample) => !sample.pinned && !hasPositive(sample));
  const rules = [...new Set(cleanOnly.flatMap((sample) => [...sample.expected.keys()]))].sort(byText);
  const chosen = new Map<string, Set<string>>();
  for (const rule of rules) {
    const files = cleanOnly.filter((sample) => sample.expected.has(rule)).sort(bySize);
    const near = files.filter((sample) => nearMisses.get(rule)?.has(sampleKey(sample)) === true);
    const picked = near.length > 0 ? near.slice(0, NEAR_MISS_CAP) : files.slice(0, 1);
    const reason = near.length > 0 ? "near miss" : "smallest clean file (no near miss)";
    for (const sample of files) {
      if (picked.includes(sample)) {
        record(sample, rule, { kept: true, reason });
        chosen.set(sampleKey(sample), (chosen.get(sampleKey(sample)) ?? new Set()).add(rule));
      } else record(sample, rule, { kept: false, reason: near.includes(sample) ? "over the cap" : "not relevant" });
    }
  }
  const kept = cleanOnly.flatMap((sample) => {
    const rulesKept = chosen.get(sampleKey(sample));
    if (rulesKept === undefined) return [];
    return [{ ...sample, expected: new Map([...sample.expected].filter(([rule]) => rulesKept.has(rule))) }];
  });
  return { samples: [...whole, ...kept].sort((a, b) => byText(sampleKey(a), sampleKey(b))), clean };
}
