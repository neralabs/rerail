/**
 * One repository Rerail knows (`GET /api/repositories`) as a choice: the top bar's repository menu and the page
 * that asks for a repository both list them this way. Choosing one is the common `pickrepo` action. What a
 * repository holds is its rule count (`repo-rules.js`), not whether it was set up: set-up can record no rules.
 */
import { h, icon } from "../dom.js";
import { fmt, plural } from "../format.js";
import { data, runningCount } from "../state.js";
import { repoRuleCounts, repoTag } from "../repo-rules.js";

/**
 * `{ total, on }` for repository `repo`: its own rules as last read (`loadRepoRuleCounts`), else for the repository
 * the page shows the rules it polled; `null` while unknown.
 */
export function repoCounts(repo) {
  const read = repoRuleCounts(repo.repositoryKey);
  if (read !== null) return read;
  const shown = data.loaded && repo.repositoryKey === data.repository?.repositoryKey;
  return shown ? { total: data.rules.size, on: runningCount() } : null;
}

/** "12 rules on · 2 checkouts", "Off · 12 rules", "No rules added"; without known counts only On or Off. */
export function repoMeta(repo) {
  const counts = repoCounts(repo);
  const checkouts = repo.checkoutCount > 1 ? ` · ${fmt(repo.checkoutCount)} checkouts` : "";
  if (counts === null) return `${repo.started ? "On" : "Off"}${checkouts}`;
  if (counts.total > 0 && !repo.started) return `Off · ${fmt(counts.total)} ${plural(counts.total, "rule")}${checkouts}`;
  return `${repoTag(counts)}${checkouts}`;
}

/** A repository as an option; `selected` marks the one the page shows. */
export function repoOption(repo, selected) {
  return h("button", { role: "option", "aria-selected": String(selected), "data-act": "pickrepo", "data-i": repo.repositoryKey, title: repo.repositoryKey },
    h("b", null, repo.name), icon("check", 14), h("span", null, repoMeta(repo)));
}
