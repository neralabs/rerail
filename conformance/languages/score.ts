import type { Divergence, FINDING_PARTS } from "./pack.js";
import { compareDeterministicStrings } from "../../src/stable-json.js";

/** A finding's position in upstream coordinates: 1-based line, 1-based code-point column. */
export interface Point {
  path: string;
  line: number;
  column?: number;
}

/**
 * A finding's whole position in upstream coordinates: its start, and the end just past its last character (1-based
 * line, 1-based exclusive code-point column). Without `endLine`, the end column lies on a line the tool does not report.
 */
export interface Position extends Point {
  endLine?: number;
  endColumn?: number;
}

/** The parts of a position a recording carries for a rule; the line always. */
export type Part = "line" | (typeof FINDING_PARTS)[number];

export type Side = "fp" | "fn";

export interface ExplainedPoint {
  side: Side;
  point: Point;
  divergence: Divergence;
}

export interface SourceScore {
  tp: number;
  /** Rerail findings the upstream tool does not report, not explained by a divergence. */
  falsePositives: Point[];
  /** Upstream findings Rerail does not report, not explained by a divergence. */
  falseNegatives: Point[];
  explained: ExplainedPoint[];
}

const comparePoints = (left: Point, right: Point): number =>
  compareDeterministicStrings(left.path, right.path) || left.line - right.line || (left.column ?? 0) - (right.column ?? 0);

/**
 * Multiset agreement of `actual` (Rerail) against `expected` (upstream) at `match` granularity; every unmatched point
 * on a line a divergence names is explained by it instead of counting as FP or FN. A `rerail-unknown` divergence
 * explains only a missed finding: Rerail declining to decide never excuses a finding it claims.
 */
export function scoreSource(
  expected: readonly Point[],
  actual: readonly Point[],
  match: "line" | "line+column",
  divergences: readonly Divergence[],
): SourceScore {
  const key = (point: Point): string => (match === "line" ? `${point.path}\0${point.line}` : `${point.path}\0${point.line}\0${point.column ?? 0}`);
  const pending = new Map<string, Point[]>();
  for (const point of actual) {
    const bucket = pending.get(key(point));
    if (bucket === undefined) pending.set(key(point), [point]);
    else bucket.push(point);
  }
  let tp = 0;
  const missed: Point[] = [];
  for (const point of expected) {
    const bucket = pending.get(key(point));
    if (bucket !== undefined && bucket.length > 0) {
      bucket.pop();
      tp += 1;
    } else missed.push(point);
  }
  const extra = [...pending.values()].flat();

  const explained: ExplainedPoint[] = [];
  const unexplained = (side: Side, points: readonly Point[]): Point[] =>
    points
      .filter(point => {
        const divergence = divergences.find(
          entry => entry.path === point.path && entry.line === point.line && (side === "fn" || entry.kind !== "rerail-unknown"),
        );
        if (divergence === undefined) return true;
        explained.push({ side, point, divergence });
        return false;
      })
      .sort(comparePoints);
  const falsePositives = unexplained("fp", extra);
  const falseNegatives = unexplained("fn", missed);
  explained.sort((left, right) => comparePoints(left.point, right.point) || compareDeterministicStrings(left.side, right.side));
  return { tp, falsePositives, falseNegatives, explained };
}

/**
 * Precision leaves out explained FPs (Rerail is right, or the comparison is not like-for-like). Recall counts an
 * explained FN against Rerail unless the upstream finding itself is wrong: a reference positive Rerail does not claim
 * is a recall loss however well it is understood.
 */
export function precisionRecall(score: SourceScore): { precision: number | null; recall: number | null } {
  const lost = score.explained.filter(entry => entry.side === "fn" && entry.divergence.kind !== "upstream-wrong").length;
  const precisionDenominator = score.tp + score.falsePositives.length;
  const recallDenominator = score.tp + score.falseNegatives.length + lost;
  return {
    precision: precisionDenominator === 0 ? null : score.tp / precisionDenominator,
    recall: recallDenominator === 0 ? null : score.tp / recallDenominator,
  };
}

/** One port's fixtures, as check 4 compares them beyond the declared match. */
export interface PortFixtures {
  /** Upstream's findings of the rule, with the parts the recording carries. */
  expected: readonly Position[];
  /** Rerail's findings, with every part. */
  actual: readonly Position[];
  match: "line" | "line+column";
  /** The parts the recording carries for the rule, in `["line", ...FINDING_PARTS]` order. */
  parts: readonly Part[];
  /** The divergences that apply to the rule on the fixtures. */
  divergences: readonly Divergence[];
  /** The fixture units the rule was evaluated on, each as its paths; a path is in one unit. */
  units: readonly (readonly string[])[];
  /** The rule's failures that are not informational, by unit (index into `units`); `path` when the engine gave one. */
  failures: readonly { unit: number; path?: string; text: string }[];
}

/** A fixture file where the port does not agree completely: the findings left unpaired at every part, and failures. */
export interface PortDifference {
  path: string;
  missing: Omit<Position, "path">[];
  extra: Omit<Position, "path">[];
  failures: string[];
}

export interface PortComparison {
  /** Pairs equal at the declared match, as `scoreSource` pairs them. */
  pointsMatched: number;
  /** Pairs equal at every carried part. */
  matched: number;
  /** Upstream's findings and Rerail's left unpaired at every carried part, before any divergence explains them. */
  missing: Position[];
  extra: Position[];
  /** Of those, the ones a divergence names the line of: a miss by any kind, a Rerail finding by any but rerail-unknown. */
  explained: number;
  /** ports:span: pairs equal at the declared match that differ at a finer part, which no divergence explains. */
  span: { expected: Position; actual: Position }[];
  /** ports:unknown-misses: upstream findings missed at the declared match that only a rerail-unknown divergence explains. */
  unknownMisses: Point[];
  /** ports:evidence: the units where the port agrees completely, and the upstream findings in them. */
  agreeing: number;
  evidence: number;
  /** In path order. */
  differences: PortDifference[];
}

/** `position` (a Rerail finding, or a recorded one) with only `parts`, always its path and line. */
export function keepParts(
  position: Pick<Position, "path" | "line"> & { readonly [part in (typeof FINDING_PARTS)[number]]?: number | undefined },
  parts: readonly Part[],
): Position {
  const kept: Position = { path: position.path, line: position.line };
  for (const part of parts) {
    if (part === "line") continue;
    const value = position[part];
    if (value !== undefined) kept[part] = value;
  }
  return kept;
}

const comparePositions = (left: Position, right: Position): number =>
  comparePoints(left, right) || (left.endLine ?? 0) - (right.endLine ?? 0) || (left.endColumn ?? 0) - (right.endColumn ?? 0);

/** The positions of `left` that `right` does not account for, compared as multisets by `key`. */
function unmatched(left: readonly Position[], right: readonly Position[], key: (position: Position) => string): Position[] {
  const pool = new Map<string, number>();
  for (const position of right) pool.set(key(position), (pool.get(key(position)) ?? 0) + 1);
  return left.filter(position => {
    const count = pool.get(key(position)) ?? 0;
    if (count === 0) return true;
    pool.set(key(position), count - 1);
    return false;
  });
}

const group = <T>(items: readonly T[], key: (item: T) => string): Map<string, T[]> => {
  const groups = new Map<string, T[]>();
  for (const item of items) {
    const bucket = groups.get(key(item));
    if (bucket === undefined) groups.set(key(item), [item]);
    else bucket.push(item);
  }
  return groups;
};

/**
 * Check 4 beyond the declared match (V1's Gate 5 span semantics), on the fixtures. Both sides are compared as multisets
 * at every part the recording carries, even a finer one than `match` (a `line` port's columns). Within each group of
 * findings equal at `match`, the pairs left over once every exact pair is taken differ only at a finer part: those are
 * ports:span's, and the findings beyond them are ports:points' (`scoreSource`'s FN and FP), so one cause fails one
 * sub-check. Divergences excuse a span difference as they excuse points, by line: a Rerail finding is never excused
 * by rerail-unknown. A unit agrees completely when nothing in it is unpaired at every part (so no divergence explains
 * anything in it) and the rule has no failure there but `source-partial`.
 */
export function comparePort(input: PortFixtures): PortComparison {
  const { match, parts, divergences } = input;
  const finer = parts.filter(part => part !== "line");
  const fullKey = (position: Position): string => [position.path, position.line, ...finer.map(part => position[part] ?? "")].join("\0");
  const matchParts: Part[] = match === "line+column" && parts.includes("column") ? ["line", "column"] : ["line"];
  const matchKey = (position: Position): string => fullKey(keepParts(position, matchParts));
  const expected = input.expected.map(position => keepParts(position, parts));
  const actual = input.actual.map(position => keepParts(position, parts));
  const missing = unmatched(expected, actual, fullKey).sort(comparePositions);
  const extra = unmatched(actual, expected, fullKey).sort(comparePositions);

  const named = (position: Position, side: Side): boolean =>
    divergences.some(entry => entry.path === position.path && entry.line === position.line && (side === "fn" || entry.kind !== "rerail-unknown"));
  const span: PortComparison["span"] = [];
  const extraByMatch = group(extra, matchKey);
  for (const [key, misses] of group(missing, matchKey)) {
    const claims = extraByMatch.get(key) ?? [];
    for (let index = 0; index < Math.min(misses.length, claims.length); index += 1) {
      if (!named(claims[index]!, "fp")) span.push({ expected: misses[index]!, actual: claims[index]! });
    }
  }

  const points = scoreSource(
    expected.map(position => keepParts(position, matchParts)),
    actual.map(position => keepParts(position, matchParts)),
    match,
    divergences,
  );
  const unknownMisses = points.explained
    .filter(({ side, point }) => side === "fn" && divergences.every(entry => entry.path !== point.path || entry.line !== point.line || entry.kind === "rerail-unknown"))
    .map(({ point }) => point);

  const unitOf = new Map(input.units.flatMap((paths, unit) => paths.map(path => [path, unit] as const)));
  const disagreeing = new Set(input.failures.map(failure => failure.unit));
  for (const position of [...missing, ...extra]) {
    const unit = unitOf.get(position.path);
    if (unit !== undefined) disagreeing.add(unit);
  }
  const agrees = (unit: number | undefined): boolean => unit !== undefined && !disagreeing.has(unit);
  const agreeing = input.units.filter((_paths, unit) => agrees(unit)).length;
  const evidence = expected.filter(position => agrees(unitOf.get(position.path))).length;

  const failuresByPath = group(
    input.failures.flatMap(failure => {
      const path = failure.path ?? input.units[failure.unit]?.[0];
      return path === undefined ? [] : [{ path, text: failure.text }];
    }),
    failure => failure.path,
  );
  const missingByPath = group(missing, position => position.path);
  const extraByPath = group(extra, position => position.path);
  const withoutPath = ({ path: _path, ...rest }: Position): Omit<Position, "path"> => rest;
  const differences = [...new Set([...missingByPath.keys(), ...extraByPath.keys(), ...failuresByPath.keys()])]
    .sort(compareDeterministicStrings)
    .map(path => ({
      path,
      missing: (missingByPath.get(path) ?? []).map(withoutPath),
      extra: (extraByPath.get(path) ?? []).map(withoutPath),
      failures: (failuresByPath.get(path) ?? []).map(failure => failure.text),
    }));

  return {
    pointsMatched: points.tp,
    matched: expected.length - missing.length,
    missing,
    extra,
    explained: missing.filter(position => named(position, "fn")).length + extra.filter(position => named(position, "fp")).length,
    span,
    unknownMisses,
    agreeing,
    evidence,
    differences,
  };
}
