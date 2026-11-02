import type { Divergence } from "./pack.js";
import { compareDeterministicStrings } from "../../src/engine/index.js";

/** A finding's position in upstream coordinates: 1-based line, 1-based code-point column. */
export interface Point {
  path: string;
  line: number;
  column?: number;
}

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
  const falsePositives = unexplained("fp", extra, divergences, explained);
  const falseNegatives = unexplained("fn", missed, divergences, explained);
  return { tp, falsePositives, falseNegatives, explained: sortExplained(explained) };
}

/**
 * The points no divergence explains, sorted; each explained one goes to `explained` instead. A divergence explains every
 * unmatched point on the line it names. A `rerail-unknown` divergence explains only a missed finding: Rerail declining
 * to decide never excuses a finding it claims.
 */
function unexplained(side: Side, points: readonly Point[], divergences: readonly Divergence[], explained: ExplainedPoint[]): Point[] {
  return points
    .filter(point => {
      const divergence = divergences.find(entry => entry.path === point.path && entry.line === point.line && (side === "fn" || entry.kind !== "rerail-unknown"));
      if (divergence === undefined) return true;
      explained.push({ side, point, divergence });
      return false;
    })
    .sort(comparePoints);
}

const sortExplained = (explained: ExplainedPoint[]): ExplainedPoint[] =>
  explained.sort((left, right) => comparePoints(left.point, right.point) || compareDeterministicStrings(left.side, right.side));

/** One real-code sample judged for a rule: its evaluation path and its labelled lines; none means clean for the rule. */
export interface JudgedSample {
  path: string;
  lines: readonly number[];
}

export interface SamplesScore extends SourceScore {
  /** Reports on a labelled sample's unlabelled lines: possibly real violations the labels missed, so reported, never failed. */
  reviewed: Point[];
}

/**
 * A rule's judgement on real-code samples, by intent and at line granularity: a label is caught when the rule reports
 * on its line (a TP) and missed otherwise (an FN); a report on a clean sample is an FP; a report on a labelled sample's
 * unlabelled line is only reviewed. Divergences explain FPs and FNs as `scoreSource`'s do. `actual` holds line points
 * on the judged samples only.
 */
export function scoreSamples(samples: readonly JudgedSample[], actual: readonly Point[], divergences: readonly Divergence[]): SamplesScore {
  const reported = new Map<string, Point[]>();
  for (const point of actual) reported.set(point.path, [...(reported.get(point.path) ?? []), point]);
  let tp = 0;
  const missed: Point[] = [];
  const cleanFindings: Point[] = [];
  const reviewed: Point[] = [];
  for (const { path, lines } of samples) {
    const points = reported.get(path) ?? [];
    if (lines.length === 0) {
      cleanFindings.push(...points);
      continue;
    }
    const reportedLines = new Set(points.map(point => point.line));
    for (const line of lines) {
      if (reportedLines.has(line)) tp += 1;
      else missed.push({ path, line });
    }
    const labelled = new Set(lines);
    reviewed.push(...points.filter(point => !labelled.has(point.line)));
  }
  const explained: ExplainedPoint[] = [];
  const falsePositives = unexplained("fp", cleanFindings, divergences, explained);
  const falseNegatives = unexplained("fn", missed, divergences, explained);
  return { tp, falsePositives, falseNegatives, explained: sortExplained(explained), reviewed: reviewed.sort(comparePoints) };
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
