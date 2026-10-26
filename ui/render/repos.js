/**
 * One repository Rerail knows (`GET /api/repositories`) as a choice: the top bar's repository menu and the page
 * that asks for a repository both list them this way. Choosing one is the common `pickrepo` action. Under the name:
 * whether it was set up, then its rule count (`repo-rules.js`), since set-up can record no rules.
 */
import { h, icon } from "../dom.js";
import { data, runningCount } from "../state.js";
import { repoLine, repoRuleCounts } from "../repo-rules.js";

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

/** The line under the repository's name (`repoLine`), from the counts the page holds for it. */
export const repoMeta = repo => repoLine(repo, repoCounts(repo));

/** A repository as an option; `selected` marks the one the page shows. */
export function repoOption(repo, selected) {
  return h("button", { role: "option", "aria-selected": String(selected), "data-act": "pickrepo", "data-i": repo.repositoryKey, title: repo.repositoryKey },
    h("b", null, repo.name), icon("check", 14), h("span", null, repoMeta(repo)));
}
