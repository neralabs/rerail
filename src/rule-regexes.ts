import { loadNativeBinding } from "./native.js";
import { isSimilarityMatch, type CosineRule, type MatchCondition } from "./rule-schema.js";

export interface RegexIssue {
  /** Where the pattern sits, from the rule root: `["match", "all", 1, "regex"]`. */
  path: (string | number)[];
  message: string;
}

function* regexes(condition: MatchCondition, path: (string | number)[]): Generator<[string, (string | number)[]]> {
  if ("regex" in condition) yield [condition.regex, [...path, "regex"]];
  else if ("all" in condition) for (const [index, item] of condition.all.entries()) yield* regexes(item, [...path, "all", index]);
  else if ("any" in condition) for (const [index, item] of condition.any.entries()) yield* regexes(item, [...path, "any", index]);
  else if ("not" in condition) yield* regexes(condition.not, [...path, "not"]);
}

/**
 * Patterns in a structural match that the kernel would refuse to compile. The kernel owns the regex
 * dialect and `maxRegexPatternBytes`; this lives apart from `rule-schema.ts` because that module is
 * shared with browser bundles. Without the native binding nothing is reported, and the kernel rejects
 * the pattern when rules load.
 */
export function kernelRegexIssues(rule: CosineRule): RegexIssue[] {
  if (isSimilarityMatch(rule.match)) return [];
  let binding: ReturnType<typeof loadNativeBinding>;
  try {
    binding = loadNativeBinding();
  } catch {
    return [];
  }
  const issues: RegexIssue[] = [];
  for (const [pattern, path] of regexes(rule.match, ["match"])) {
    try {
      binding.validateRegex(pattern);
    } catch (error) {
      issues.push({ path, message: error instanceof Error ? error.message : String(error) });
    }
  }
  return issues;
}

/** Throw on the first pattern the kernel would refuse, located like `parseCosineRule` errors. */
export function assertKernelRegexes(rule: CosineRule, root = "rule"): void {
  const [issue] = kernelRegexIssues(rule);
  if (issue !== undefined) throw new TypeError(`${[root, ...issue.path.map(String)].join(".")} ${issue.message}`);
}
