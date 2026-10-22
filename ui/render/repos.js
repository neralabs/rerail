/**
 * One repository Rerail knows (`GET /api/repositories`) as a choice: the top bar's repository menu and the page
 * that asks for a repository both list them this way. Choosing one is the common `pickrepo` action.
 */
import { h, icon } from "../dom.js";
import { fmt, plural } from "../format.js";
import { data, runningCount } from "../state.js";

/** "Set up · Started · 12 rules on · 2 checkouts", or "Not set up · nothing runs". */
export function repoMeta(repo) {
  if (!repo.setUp) return "Not set up · nothing runs";
  const current = repo.repositoryKey === data.repository?.repositoryKey;
  const on = current ? ` · ${fmt(runningCount())} ${plural(runningCount(), "rule")} on` : "";
  const checkouts = repo.checkoutCount > 1 ? ` · ${fmt(repo.checkoutCount)} checkouts` : "";
  return `Set up · ${repo.started ? "Started" : "Stopped"}${on}${checkouts}`;
}

/** A repository as an option; `selected` marks the one the page shows. */
export function repoOption(repo, selected) {
  return h("button", { role: "option", "aria-selected": String(selected), "data-act": "pickrepo", "data-i": repo.repositoryKey, title: repo.repositoryKey },
    h("b", null, repo.name), icon("check", 14), h("span", null, repoMeta(repo)));
}
