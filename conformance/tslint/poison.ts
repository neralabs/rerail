/**
 * Identifier-poisoning gate: scores every port on the normal fixtures and on the renamed fixtures (`rename.ts`) and
 * reports each rule whose score moves. A port that implements structure scores identically; one that matches names only
 * the fixtures use (`foo`, `MyClass`) loses TPs or gains FPs. Parse checks per file: TypeScript's syntax diagnostics
 * (in the runner) and the kernel's `source-partial` error-region message must be unchanged by the rename.
 *
 *   just rerail-conformance --rename-identifiers                 every rule, table of rules whose score changed
 *   just rerail-conformance --rename-identifiers --rule <name>   one rule's detailed diff on the renamed fixtures
 */
import type { RuleInfo } from "./fixtures.js";
import { evaluateRule, type FileResult, type RuleResult } from "./runner.js";

/**
 * Rules whose TSLint semantics compare names lexically (alphabetical member/key/import order): any rename can reorder
 * names, so their score may move without poisoning. The table marks their rows `lexical order`.
 */
export const NAME_ORDER_RULES: ReadonlyMap<string, string> = new Map([
  ["member-ordering", "alphabetize option sorts member names"],
  ["object-literal-sort-keys", "sorts object keys"],
  ["ordered-imports", "sorts import names and module paths"],
]);

export interface Score {
  truePositives: number;
  falsePositives: number;
  falseNegatives: number;
  engineFailures: number;
}

export interface RenameComparison {
  rule: string;
  verdict: RuleResult["verdict"];
  normal: Score;
  renamed: Score;
  /** Repaired-fixture score, when the rule has repairs. */
  normalRepaired: Score | undefined;
  renamedRepaired: Score | undefined;
  changed: boolean;
  /** Renamer problems: parse changes (TypeScript or kernel) and exhausted names. */
  renamerIssues: string[];
  /** Expected failures missed only on the renamed fixtures, and findings extra only there (`file line:col`). */
  lostOnRename: string[];
  gainedOnRename: string[];
}

const score = (result: Score): Score => ({
  truePositives: result.truePositives,
  falsePositives: result.falsePositives,
  falseNegatives: result.falseNegatives,
  engineFailures: result.engineFailures,
});

const sameScore = (left: Score | undefined, right: Score | undefined): boolean =>
  left === right ||
  (left !== undefined &&
    right !== undefined &&
    left.truePositives === right.truePositives &&
    left.falsePositives === right.falsePositives &&
    left.falseNegatives === right.falseNegatives &&
    left.engineFailures === right.engineFailures);

const fileKey = (result: FileResult): string => `${result.variant}/${result.lintPath}`;

function positions(results: FileResult[], pick: (result: FileResult) => Array<{ startLine: number; startCol: number }>): Set<string> {
  return new Set(results.flatMap(result => pick(result).map(position => `${fileKey(result)} ${position.startLine + 1}:${position.startCol + 1}`)));
}

export function compareRename(normal: RuleResult, renamed: RuleResult): RenameComparison {
  const renamerIssues: string[] = [];
  const normalByFile = new Map(normal.results.map(result => [fileKey(result), result]));
  for (const result of renamed.results) {
    for (const issue of result.renameIssues ?? []) renamerIssues.push(`${fileKey(result)}: ${issue}`);
    const before = normalByFile.get(fileKey(result));
    if (before !== undefined && before.partial !== result.partial) renamerIssues.push(`${fileKey(result)}: kernel parse changed: ${before.partial ?? "clean"} -> ${result.partial ?? "clean"}`);
  }
  const missedBefore = positions(normal.results, result => result.missed);
  const extraBefore = positions(normal.results, result => result.extra);
  const lostOnRename = [...positions(renamed.results, result => result.missed)].filter(key => !missedBefore.has(key));
  const gainedOnRename = [...positions(renamed.results, result => result.extra)].filter(key => !extraBefore.has(key));
  const normalRepaired = normal.repaired === undefined ? undefined : score(normal.repaired);
  const renamedRepaired = renamed.repaired === undefined ? undefined : score(renamed.repaired);
  return {
    rule: normal.rule,
    verdict: normal.verdict,
    normal: score(normal),
    renamed: score(renamed),
    normalRepaired,
    renamedRepaired,
    changed: !sameScore(score(normal), score(renamed)) || !sameScore(normalRepaired, renamedRepaired),
    renamerIssues,
    lostOnRename,
    gainedOnRename,
  };
}

export async function renameCheck(rule: RuleInfo): Promise<RenameComparison> {
  return compareRename(await evaluateRule(rule, 1), await evaluateRule(rule, 1, { renameIdentifiers: true }));
}

const triple = (value: Score): string => `${value.truePositives}/${value.falsePositives}/${value.falseNegatives}${value.engineFailures > 0 ? ` +${value.engineFailures}e` : ""}`;

export function renameMarkdown(comparisons: RenameComparison[]): string {
  const changed = comparisons.filter(comparison => comparison.changed);
  const unexplained = changed.filter(comparison => !NAME_ORDER_RULES.has(comparison.rule));
  const issues = comparisons.filter(comparison => comparison.renamerIssues.length > 0);
  const lines = [
    "# TSLint ports on renamed fixtures",
    "",
    `${comparisons.length} rules; ${changed.length} change score when user identifiers are renamed (${unexplained.length} not explained by lexical name order); ${issues.length} with renamer issues.`,
    "",
  ];
  if (changed.length > 0) {
    lines.push("| rule | verdict | TP/FP/FN normal | TP/FP/FN renamed | repaired normal | repaired renamed | note |", "|---|---|---|---|---|---|---|");
    for (const row of changed) {
      const repaired = (value: Score | undefined): string => (value === undefined ? "" : triple(value));
      const note = NAME_ORDER_RULES.has(row.rule) ? `lexical order: ${NAME_ORDER_RULES.get(row.rule)!}` : "";
      lines.push(`| ${row.rule} | ${row.verdict} | ${triple(row.normal)} | ${triple(row.renamed)} | ${repaired(row.normalRepaired)} | ${repaired(row.renamedRepaired)} | ${note} |`);
    }
    lines.push("");
    for (const row of changed) {
      lines.push(`## ${row.rule}`, "");
      for (const key of row.lostOnRename.slice(0, 20)) lines.push(`- missed only when renamed: ${key}`);
      for (const key of row.gainedOnRename.slice(0, 20)) lines.push(`- extra only when renamed: ${key}`);
      lines.push("");
    }
  }
  for (const row of issues) {
    lines.push(`## renamer issues: ${row.rule}`, "", ...row.renamerIssues.map(issue => `- ${issue}`), "");
  }
  return `${lines.join("\n")}\n`;
}
