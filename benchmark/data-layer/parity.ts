// SQL against native, line by line: the findings and failures of one evaluation as `engine.ts` formats them, and the
// comparison the loop and the mutation stage both judge parity by.
import { compareStrings, failureLine, violationLine, type EngineResult } from "./engine.js";

/** The SQL engine leaves native's work-budget failures alone: those (rule, clause, file)s are not compared. */
export const BUDGET_FAILURE = "budget-exceeded";

export function linesOf(result: EngineResult): string[] {
  return [...result.violations.map(violationLine), ...result.failures.map(failureLine)].sort(compareStrings);
}

export interface Line {
  text: string;
  kind: string;
  rule: string;
  clause: string;
  code: string;
  path: string;
}

export function parseLine(text: string): Line {
  const [kind = "", rule = "", clause = "", code = "", path = ""] = text.split("\t");
  return { text, kind, rule, clause, code, path };
}

export interface ClauseParity {
  clause: string;
  compared: number;
  /** (rule, clause, file)s native ran out of its work budget on: not compared. */
  budgetExcluded: string[];
  files: { path: string; onlyNative: string[]; onlySql: string[] }[];
}

/** One clause's SQL lines against native's, apart from the files where native ran out of its work budget. */
export function compareLines(clause: string, native: readonly string[], sql: readonly string[]): ClauseParity {
  const key = (line: Line) => `${line.rule}\t${line.clause}\t${line.path}`;
  const parsedNative = native.map(parseLine);
  const excluded = new Set(parsedNative.filter(line => line.kind === "failure" && line.code === BUDGET_FAILURE).map(key));
  const left = parsedNative.filter(line => !excluded.has(key(line)));
  const right = sql.map(parseLine).filter(line => !excluded.has(key(line)));
  const count = (lines: Line[]) => {
    const counts = new Map<string, number>();
    for (const line of lines) counts.set(line.text, (counts.get(line.text) ?? 0) + 1);
    return counts;
  };
  const leftCounts = count(left);
  const rightCounts = count(right);
  const files = new Map<string, ClauseParity["files"][number]>();
  const file = (line: Line) => {
    let found = files.get(line.path);
    if (found === undefined) files.set(line.path, found = { path: line.path, onlyNative: [], onlySql: [] });
    return found;
  };
  for (const line of left) if ((rightCounts.get(line.text) ?? 0) < (leftCounts.get(line.text) ?? 0)) file(line).onlyNative.push(line.text);
  for (const line of right) if ((leftCounts.get(line.text) ?? 0) < (rightCounts.get(line.text) ?? 0)) file(line).onlySql.push(line.text);
  for (const found of files.values()) {
    found.onlyNative = [...new Set(found.onlyNative)];
    found.onlySql = [...new Set(found.onlySql)];
  }
  return { clause, compared: left.length, budgetExcluded: [...excluded].sort(compareStrings), files: [...files.values()].sort((a, b) => compareStrings(a.path, b.path)) };
}
