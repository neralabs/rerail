import type { Check } from "./check.js";

/**
 * JSON with object keys sorted at every level (code-unit order); rejects non-finite numbers so the text is a stable
 * measure of a document's size.
 */
export function canonicalJson(value: unknown): string {
  if (value === null || typeof value === "boolean" || typeof value === "string") return JSON.stringify(value);
  if (typeof value === "number") {
    if (!Number.isFinite(value)) throw new Error("Cannot canonicalize a non-finite number");
    return JSON.stringify(value);
  }
  if (Array.isArray(value)) return `[${value.map(canonicalJson).join(",")}]`;
  if (typeof value === "object") {
    const record = value as Record<string, unknown>;
    return `{${Object.keys(record).sort().map((key) => `${JSON.stringify(key)}:${canonicalJson(record[key])}`).join(",")}}`;
  }
  throw new Error(`Cannot canonicalize ${typeof value}`);
}

/** compact's flag: 15 KiB of shipped bytes. A signal to inspect, not a limit. */
export const SIZE_FLAG_BYTES = 15 * 1024;
/** A flagged rule is an expansion when at least this share of its clauses is structure written out over and over. */
export const REPEATED_SHARE_LIMIT = 1 / 3;
/** A flagged rule writes repetition out when one node kind occurs this many times along a single nesting path (`A{n}`). */
export const NESTING_RUN_LIMIT = 3;
/** Subtrees smaller than this (canonical bytes) are building blocks such as `{"kind":"identifier"}`, not structure. */
const MIN_REPEATED_BYTES = 64;
/** A group of same-shaped siblings smaller than this in total (canonical bytes) is too small to be expansion. */
const MIN_GROUP_BYTES = 192;

const COMBINATORS = ["all", "any"] as const;
const RELATIONS = ["has", "inside", "follows", "precedes"] as const;
const WORD = /[A-Za-z_]\w*/gu;

export interface SizeMetrics {
  rawBytes: number;
  canonicalBytes: number;
  flagged: boolean;
  /**
   * Canonical bytes of structure written out over and over: the second and later items of a product (three or more
   * same-shaped siblings in an array or utility map whose varying words each recur across items), and every
   * verbatim copy of a subtree after its first. Violation blocks never count.
   */
  repeatedBytes: number;
  repeatedShare: number;
  /** The most times one node kind (or pattern) definitely occurs along a single nesting path, through utilities and `count`. */
  nestingRun: number;
  /** Each node kind (`pattern:<text>` for a pattern) whose run reaches `NESTING_RUN_LIMIT`, with its run. */
  longRuns: NestingRuns;
  /** Utilities that reach themselves through `matches`. Reported, not judged: recursion is how the DSL walks trees. */
  recursiveUtils: string[];
}

/** Runs of one node kind along a nesting path, by kind: a pattern's key is `pattern:<text>`. */
export type NestingRuns = Record<string, number>;

/**
 * A rule's declared exemption from no-written-out-repetition: `reason`, why its runs are not a copy written out, and
 * `runs`, the runs it excuses, which must be the runs the check measures (`NestingRuns`, those at the limit or over).
 */
export interface RepetitionExemption {
  reason: string;
  runs: NestingRuns;
}

export interface CompactResult {
  checks: Check[];
  metrics: SizeMetrics;
}

function isRecord(value: unknown): value is Record<string, unknown> {
  return value !== null && typeof value === "object" && !Array.isArray(value);
}

/** A subtree without its violation blocks: a clause's message is why it exists, not structure. */
function withoutViolations(value: unknown): unknown {
  if (Array.isArray(value)) return value.map(withoutViolations);
  if (isRecord(value)) return Object.fromEntries(Object.entries(value).filter(([key]) => key !== "violation").map(([key, child]) => [key, withoutViolations(child)]));
  return value;
}

function maskStrings(value: unknown): unknown {
  if (typeof value === "string") return "*";
  if (Array.isArray(value)) return value.map(maskStrings);
  if (isRecord(value)) return Object.fromEntries(Object.entries(value).map(([key, child]) => [key, maskStrings(child)]));
  return value;
}

function strings(value: unknown): string[] {
  if (typeof value === "string") return [value];
  if (Array.isArray(value)) return value.flatMap(strings);
  if (isRecord(value)) return Object.values(value).flatMap(strings);
  return [];
}

/**
 * True when same-shaped siblings are a product written out cell by cell: every word in the strings that vary between
 * them appears in at least two of them. A table of distinct cases (one check per deprecated API) has words of its own.
 */
function isProduct(items: unknown[]): boolean {
  const columns = items.map(strings);
  const varying = columns[0]!.map((_, index) => new Set(columns.map((column) => column[index])).size > 1);
  const itemsWithWord = new Map<string, number>();
  for (const column of columns) {
    const words = new Set(column.filter((_, index) => varying[index]).flatMap((text) => text.match(WORD) ?? []));
    for (const word of words) itemsWithWord.set(word, (itemsWithWord.get(word) ?? 0) + 1);
  }
  return itemsWithWord.size > 0 && [...itemsWithWord.values()].every((count) => count >= 2);
}

function repeatedStructureBytes(clauses: unknown): number {
  const verbatim = new Set<string>();
  let repeated = 0;
  const siblings = (items: unknown[]): void => {
    const groups = new Map<string, unknown[]>();
    for (const item of items) {
      if (!isRecord(item)) continue;
      const shape = canonicalJson(maskStrings(withoutViolations(item)));
      groups.set(shape, [...(groups.get(shape) ?? []), item]);
    }
    const counted = new Set<unknown>();
    for (const group of groups.values()) {
      const bytes = group.map((item) => canonicalJson(withoutViolations(item)).length);
      if (group.length < 3 || bytes.reduce((sum, value) => sum + value, 0) < MIN_GROUP_BYTES || !isProduct(group.map(withoutViolations))) continue;
      group.slice(1).forEach((item, index) => {
        repeated += bytes[index + 1]!;
        counted.add(item);
      });
    }
    for (const item of items) if (!counted.has(item)) visit(item);
  };
  const visit = (value: unknown): void => {
    if (Array.isArray(value)) {
      siblings(value);
      return;
    }
    if (!isRecord(value)) return;
    const text = canonicalJson(withoutViolations(value));
    if (text.length >= MIN_REPEATED_BYTES) {
      if (verbatim.has(text)) {
        repeated += text.length;
        return;
      }
      verbatim.add(text);
    }
    for (const [key, child] of Object.entries(value)) {
      if (key === "violation") continue;
      // A utility map's entries are siblings, as an array's items are.
      if (key === "utils" && isRecord(child)) siblings(Object.values(child));
      else visit(child);
    }
  };
  visit(clauses);
  return repeated;
}

function nodeLabel(node: Record<string, unknown>): string | undefined {
  if (typeof node.kind === "string") return node.kind;
  if (typeof node.pattern === "string") return `pattern:${node.pattern}`;
  return undefined;
}

function utilityBody(utility: unknown): Record<string, unknown> | undefined {
  if (!isRecord(utility)) return undefined;
  return isRecord(utility.rule) && Array.isArray(utility.params) ? utility.rule : utility;
}

function calledUtilities(node: Record<string, unknown>): string[] {
  const matches = node.matches;
  if (typeof matches === "string") return [matches];
  return isRecord(matches) ? Object.keys(matches) : [];
}

/** A `define` clause's utilities and the language whose queries may call them. */
function definedUtilities(clause: unknown): { language: string; utils: Record<string, unknown> } | undefined {
  const define = isRecord(clause) ? clause.define : undefined;
  return isRecord(define) && typeof define.language === "string" && isRecord(define.utils) ? { language: define.language, utils: define.utils } : undefined;
}

/**
 * Each language's shared utilities: the `utils` of the document's `define` clauses, which every ast query of that
 * language may call as if it listed them in its own `utils`.
 */
function sharedUtilities(clauses: unknown): Map<string, Record<string, unknown>> {
  const shared = new Map<string, Record<string, unknown>>();
  for (const clause of Array.isArray(clauses) ? clauses : []) {
    const defined = definedUtilities(clause);
    if (defined !== undefined) shared.set(defined.language, { ...shared.get(defined.language), ...defined.utils });
  }
  return shared;
}

/** A query's own `utils`. */
function ownUtilities(query: Record<string, unknown>): Record<string, unknown> {
  return isRecord(query.utils) ? query.utils : {};
}

/** An ast query: a `rule` in a `language`, with optional `utils` of its own. */
function isAstQuery(value: Record<string, unknown>): value is Record<string, unknown> & { rule: Record<string, unknown>; language: string } {
  return isRecord(value.rule) && typeof value.language === "string";
}

/** A node's `any`: a one-item `any` is part of the node; the items of a larger one are its `alternatives`, each a branch. */
function soleAlternative(part: Record<string, unknown>, alternatives: Record<string, unknown>[][]): Record<string, unknown>[] {
  const items = Array.isArray(part.any) ? part.any.filter(isRecord) : [];
  if (items.length < 2) return items;
  alternatives.push(items);
  return [];
}

/** The bodies of the utilities `part` calls that the path has not `called` yet, which it marks called. */
function calledBodies(part: Record<string, unknown>, utils: Record<string, unknown>, called: Set<string>): Record<string, unknown>[] {
  const bodies: Record<string, unknown>[] = [];
  for (const name of calledUtilities(part)) {
    const body = utilityBody(utils[name]);
    if (body === undefined || called.has(name)) continue;
    called.add(name);
    bodies.push(body);
  }
  return bodies;
}

/**
 * Everything the node `start` definitely is: itself, its `all` parts, a one-item `any`, and the utilities it calls
 * that the path has not called yet; and the larger `any`s it branches on.
 */
function definiteParts(start: Record<string, unknown>, utils: Record<string, unknown>, onPath: Set<string>) {
  const definite: Record<string, unknown>[] = [];
  const alternatives: Record<string, unknown>[][] = [];
  const called = new Set(onPath);
  const queue = [start];
  while (queue.length > 0) {
    const part = queue.pop()!;
    definite.push(part);
    queue.push(...(Array.isArray(part.all) ? part.all.filter(isRecord) : []), ...soleAlternative(part, alternatives), ...calledBodies(part, utils, called));
  }
  return { definite, alternatives, called };
}

/** The nodes `part` moves to: its `has`, `inside`, `follows` and `precedes`, and their `count` forms. */
function nextNodes(part: Record<string, unknown>): Record<string, unknown>[] {
  const holders = isRecord(part.count) ? [part, part.count] : [part];
  return holders.flatMap((holder) => RELATIONS.map((relation) => holder[relation]).filter(isRecord));
}

/**
 * Each node kind's (or pattern's) longest run: the most times it occurs along a single nesting path. A path counts
 * what each node definitely is (`definiteParts`): its own kind, its `all` parts, a one-item `any`, and the utilities
 * it calls (each at most once per path, so recursion walks a tree instead of counting), its own or its language's
 * shared ones. Each alternative of a larger `any` is its own branch, and `not` says what a node is not, so neither is
 * merged into the node. `has`, `inside`, `follows`, `precedes` and their `count` forms move to the next node.
 */
function nestingRuns(clauses: unknown): Map<string, number> {
  const shared = sharedUtilities(clauses);
  const longest = new Map<string, number>();
  /** Counts each label once per node, onto the runs along the path (`counts`, and `level`'s labels already counted). */
  const countLevel = (definite: Record<string, unknown>[], counts: Map<string, number>, level: Set<string>) => {
    const next = new Map(counts);
    const here = new Set(level);
    for (const part of definite) {
      const label = nodeLabel(part);
      if (label === undefined || here.has(label)) continue;
      here.add(label);
      const run = (next.get(label) ?? 0) + 1;
      next.set(label, run);
      longest.set(label, Math.max(longest.get(label) ?? 0, run));
    }
    return { next, here };
  };
  const walkLevel = (start: Record<string, unknown>, utils: Record<string, unknown>, counts: Map<string, number>, level: Set<string>, onPath: Set<string>): void => {
    const { definite, alternatives, called } = definiteParts(start, utils, onPath);
    const { next, here } = countLevel(definite, counts, level);
    for (const node of definite.flatMap(nextNodes)) walkLevel(node, utils, next, new Set(), called);
    for (const alternative of alternatives.flat()) walkLevel(alternative, utils, next, here, called);
  };
  const queries = (value: unknown): void => {
    if (Array.isArray(value)) value.forEach(queries);
    else if (isRecord(value)) {
      // A query's own utilities and the shared ones it reaches are one namespace: the engine refuses an id in both.
      if (isAstQuery(value)) walkLevel(value.rule, { ...shared.get(value.language), ...ownUtilities(value) }, new Map(), new Set(), new Set());
      for (const [key, child] of Object.entries(value)) if (key !== "violation") queries(child);
    }
  };
  queries(clauses);
  return longest;
}

/** Utility names a rule calls, by id or by the `{ id: { PARAM: "$CAPTURE" } }` call form. */
function references(value: unknown): string[] {
  if (Array.isArray(value)) return value.flatMap(references);
  if (value === null || typeof value !== "object") return [];
  return Object.entries(value as Record<string, unknown>).flatMap(([key, child]) => {
    if (key !== "matches") return references(child);
    if (typeof child === "string") return [child];
    return child !== null && typeof child === "object" ? Object.keys(child as Record<string, unknown>) : [];
  });
}

/** Utilities that reach themselves, judged within each `utils` map: every query has its own utility namespace. */
function recursiveUtilities(clauses: unknown): string[] {
  const recursive = new Set<string>();
  const visit = (value: unknown): void => {
    if (Array.isArray(value)) {
      value.forEach(visit);
      return;
    }
    if (value === null || typeof value !== "object") return;
    const record = value as Record<string, unknown>;
    const utils = record.utils;
    if (utils !== null && typeof utils === "object" && !Array.isArray(utils)) {
      const table = utils as Record<string, unknown>;
      for (const [name, rule] of Object.entries(table)) {
        const reached = new Set<string>();
        const pending = [...references(rule)];
        while (pending.length > 0) {
          const next = pending.pop()!;
          if (reached.has(next)) continue;
          reached.add(next);
          if (table[next] !== undefined) pending.push(...references(table[next]));
        }
        if (reached.has(name)) recursive.add(name);
      }
    }
    Object.values(record).forEach(visit);
  };
  visit(clauses);
  return [...recursive].sort();
}

/** `runs` as text, by kind: `call_expression 3, pair 4`. */
function runsText(runs: NestingRuns): string {
  return Object.keys(runs)
    .sort()
    .map((kind) => `${kind} ${runs[kind]}`)
    .join(", ");
}

/** The runs the walk measured, against their limit: what a no-written-out-repetition detail reports. */
function measuredRun(metrics: SizeMetrics): string {
  const long = Object.keys(metrics.longRuns).length === 0 ? "" : `; runs of ${NESTING_RUN_LIMIT} or more: ${runsText(metrics.longRuns)}`;
  return `the most times one node kind occurs along a single nesting path: ${metrics.nestingRun} (limit ${NESTING_RUN_LIMIT})${long}`;
}

/** Whether `declared` names exactly the `measured` runs, kind and length. */
function sameRuns(declared: NestingRuns, measured: NestingRuns): boolean {
  const kinds = Object.keys(measured);
  return Object.keys(declared).length === kinds.length && kinds.every((kind) => declared[kind] === measured[kind]);
}

/**
 * A declared exemption on a flagged document whose runs reach the limit: it passes only while its runs are exactly the
 * runs measured, since a run it does not name, or a longer one, was not judged, and one it names that shrank or went
 * away no longer needs excusing.
 */
function exemptRuns(metrics: SizeMetrics, exempt: RepetitionExemption): Check {
  if (sameRuns(exempt.runs, metrics.longRuns)) return { id: "no-written-out-repetition", ok: true, detail: `exempt (${exempt.reason}); ${measuredRun(metrics)}` };
  const detail = `the exemption (${exempt.reason}) excuses ${runsText(exempt.runs)}, not the runs measured; ${measuredRun(metrics)}; set its runs to those measured if they are not copies either, or rewrite the repetition`;
  return { id: "no-written-out-repetition", ok: false, detail };
}

/**
 * no-written-out-repetition, or nothing for an unflagged document with no exemption. A declared exemption is judged
 * on the runs it excuses (`exemptRuns`), and fails as stale where the check would pass without it: under the flag, or
 * under the limit, the runs it excused are gone.
 */
function repetitionCheck(metrics: SizeMetrics, exempt: RepetitionExemption | undefined): Check[] {
  const needed = metrics.flagged && metrics.nestingRun >= NESTING_RUN_LIMIT;
  if (exempt === undefined) return metrics.flagged ? [{ id: "no-written-out-repetition", ok: !needed, detail: measuredRun(metrics) }] : [];
  if (needed) return [exemptRuns(metrics, exempt)];
  const now = metrics.flagged ? measuredRun(metrics) : `${metrics.rawBytes} bytes, under the ${SIZE_FLAG_BYTES}-byte flag`;
  return [{ id: "no-written-out-repetition", ok: false, detail: `stale exemption (${exempt.reason}): ${now}; remove the exemption` }];
}

/**
 * compact: a document over 15 KiB is flagged and inspected. It goes back for a rewrite when a third or more of its
 * clauses is structure written out over and over (three or more same-shaped siblings, or verbatim copies), or when one
 * node kind occurs three or more times along a single nesting path (repetition `count` or a regex quantifier would express).
 * Otherwise the size is justified. `repetitionExempt`, a rule's declared reason that its runs are not copies (each
 * step is a separate lookup or a different node, not one copy per nesting level), excuses the runs it names
 * (`repetitionCheck`).
 */
export function compactGate(rawBytes: number, document: { clauses?: unknown }, repetitionExempt?: RepetitionExemption): CompactResult {
  const clausesText = canonicalJson(document.clauses ?? null);
  const repeatedBytes = repeatedStructureBytes(document.clauses);
  const runs = nestingRuns(document.clauses);
  const longRuns: NestingRuns = {};
  for (const [kind, run] of runs) if (run >= NESTING_RUN_LIMIT) longRuns[kind] = run;
  const metrics: SizeMetrics = {
    rawBytes,
    canonicalBytes: canonicalJson(document).length,
    flagged: rawBytes > SIZE_FLAG_BYTES,
    repeatedBytes,
    repeatedShare: clausesText.length === 0 ? 0 : repeatedBytes / clausesText.length,
    nestingRun: Math.max(0, ...runs.values()),
    longRuns,
    recursiveUtils: recursiveUtilities(document.clauses),
  };
  if (!metrics.flagged) {
    return { checks: [{ id: "size", ok: true, detail: `${rawBytes} bytes, under the ${SIZE_FLAG_BYTES}-byte flag` }, ...repetitionCheck(metrics, repetitionExempt)], metrics };
  }
  const percent = (metrics.repeatedShare * 100).toFixed(1);
  return {
    checks: [
      { id: "size", ok: true, detail: `${rawBytes} bytes (${metrics.canonicalBytes} canonical) is over the ${SIZE_FLAG_BYTES}-byte flag; inspected` },
      {
        id: "no-combinatorial-expansion",
        ok: metrics.repeatedShare < REPEATED_SHARE_LIMIT,
        detail: `${percent}% of the clauses is structure written out over and over (limit ${(REPEATED_SHARE_LIMIT * 100).toFixed(1)}%)`,
      },
      ...repetitionCheck(metrics, repetitionExempt),
    ],
    metrics,
  };
}
