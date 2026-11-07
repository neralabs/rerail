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
