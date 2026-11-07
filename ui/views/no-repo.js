/**
 * The page without a repository in the URL (every view but Settings): the dashboard shows one repository at a time,
 * so it asks for one, listing every repository Rerail has seen with what it holds.
 */
import { h, setContent, icon } from "../dom.js";
import { fmt, plural } from "../format.js";
import { data } from "../state.js";
import { invalidate } from "../shell.js";
import { loadRepoRuleCounts } from "../repo-rules.js";
import { repoMeta, repoOption } from "../render/repos.js";

export const needs = [];

const NUMBER_WORDS = ["no", "one", "two", "three", "four", "five", "six", "seven", "eight", "nine"];

function note(seen) {
  return h("div.welcome.warn", { role: "status" }, icon("steered", 22), h("div", null,
    h("b", null, "No repo selected. Rerail’s dashboard shows one repo at a time."),
    seen === 0
      ? h("p", null, "Rerail has not seen a repo on this computer yet. Open a coding agent in one of your repos and type ",
        h("span.mono", null, "/rerail-start"), " there; the repo then shows here.")
      : h("p", null, `Open a coding agent in one of your repos and come here from there, or pick a repo below. Rerail has seen ${NUMBER_WORDS[seen] ?? fmt(seen)} ${plural(seen, "repo")} on this computer; a new one is set up the same way, with `,
        h("span.mono", null, "/rerail-start"), " in a coding agent session there.")));
}

/** The poll whose repository list the rule counts were last read for. */
let countedAt = -1;

/** Each listed repository's rules, read once per poll so the list says what each one holds. */
function loadCounts(list) {
  if (countedAt === data.updatedAt || list.length === 0) return;
  countedAt = data.updatedAt;
  loadRepoRuleCounts(list.map(r => r.repositoryKey)).then(changed => { if (changed) invalidate(); }, () => { /* the list keeps On or Off */ });
}

export function render(root) {
  const list = data.repositories;
  loadCounts(list);
  const signature = `${data.loaded}|${list.map(r => [r.repositoryKey, r.name, repoMeta(r)].join(",")).join("|")}`;
  setContent(root, signature, () => h("div.norepo", null,
    data.loaded ? note(list.length) : null,
    list.length === 0 ? (data.loaded ? null : h("div.list", null, h("div.empty", null, h("p", null, "Loading…"))))
      : h("section.group", null,
        h("div.ghead", null, h("h2", null, "Repos Rerail has seen"), h("p", null, "One repo at a time. Pick one to see its rules and history.")),
        h("div.list.repolist", { role: "listbox", "aria-label": "Repos Rerail has seen" }, list.map(repo => repoOption(repo, false))))));
}
