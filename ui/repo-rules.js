/**
 * What Settings' Repos rows and the repository choices say of a repository's rules, from its counts (`repo-rule-counts.js`
 * reads them). A repository set up with nothing added has none, so the count, not `setUp`, says whether rules were added.
 */
import { isRunning, toStoredRules } from "./derive.js";
import { fmt, plural } from "./format.js";

/** `{ total, on }` of a repository's rule entries: one rule per id however many folders add it; on when it runs. */
export function ruleCountsOf(entries) {
  const rules = [...toStoredRules(entries).values()];
  return { total: rules.length, on: rules.filter(isRunning).length };
}

/** "12 rules on", "0 rules on" when every rule is off, "No rules added" when there are none; `null` while unknown. */
export function repoTag(counts) {
  if (counts === null) return null;
  return counts.total === 0 ? "No rules added" : `${fmt(counts.on)} ${plural(counts.on, "rule")} on`;
}

/** The words about a set-up repository's rules: its tag; "Off · 12 rules" while it is off with rules; On or Off until the counts are read. */
function rulesWords(started, counts) {
  if (counts === null) return [started ? "On" : "Off"];
  if (counts.total > 0 && !started) return ["Off", `${fmt(counts.total)} ${plural(counts.total, "rule")}`];
  return [repoTag(counts)];
}

/**
 * The line under a repository's name in the repository choices: "Set up · 12 rules on · 2 checkouts", "Set up · Off ·
 * 12 rules", "Set up · No rules added", "Not set up · no rules run". One checkout goes unsaid.
 */
export function repoLine(repo, counts) {
  const checkouts = repo.checkoutCount > 1 ? [`${fmt(repo.checkoutCount)} checkouts`] : [];
  const state = repo.setUp ? ["Set up", ...rulesWords(repo.started, counts)] : ["Not set up", "no rules run"];
  return [...state, ...checkouts].join(" · ");
}
