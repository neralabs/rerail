/**
 * Whether a `.rerail` rule's files on disk are still those the catalog read, approved and placed. The runner's child
 * reads the files when it runs, so a file edited since the catalog read it (before the catalog reads it again) would
 * run as the rule approved before the edit. Such a rule is left out of the run, with a failure that says which file
 * changed.
 */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import type { AuditFailure } from "../audit/audit.js";
import type { RerailRuleEntry } from "../code-rule/code-rule-dsl-catalog.js";

/** The SHA-256 (hex) of the file at `path`; undefined where it cannot be read. */
function sha256Of(path: string): string | undefined {
  try {
    return createHash("sha256").update(readFileSync(path)).digest("hex");
  } catch {
    return undefined;
  }
}

/** The first of `rule`'s files (the rule, then each file it includes) that no longer holds what the catalog read. */
function changedFile(rule: RerailRuleEntry): string | undefined {
  return rule.files.find(file => sha256Of(file.path) !== file.sha256)?.path;
}

/** `rules` split into those whose files are as the catalog read them, and a failure for each of the others. */
export function unchangedRules(rules: readonly RerailRuleEntry[]): { rules: RerailRuleEntry[]; failures: AuditFailure[] } {
  const unchanged: RerailRuleEntry[] = [];
  const failures: AuditFailure[] = [];
  for (const rule of rules) {
    const changed = changedFile(rule);
    if (changed === undefined) unchanged.push(rule);
    else failures.push({
      ruleId: rule.id, clauseId: "run", code: "adapter-error", stage: "scan",
      message: `The .rerail rule was not run: ${changed} changed since Rerail read it. It runs again once Rerail reads the rule again (and a person approves it, where its pack needs approval).`,
    });
  }
  return { rules: unchanged, failures };
}
