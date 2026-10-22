/**
 * The page without a repository in the URL (every view but Settings): the dashboard shows one repository at a time,
 * so it asks for one, listing every repository Rerail has seen.
 */
import { h, setContent, icon } from "../dom.js";
import { data } from "../state.js";
import { repoOption } from "../render/repos.js";

export const needs = [];

export function render(root) {
  const list = data.repositories;
  const signature = `${data.loaded}|${list.map(r => [r.repositoryKey, r.name, r.setUp, r.started, r.checkoutCount].join(",")).join("|")}`;
  setContent(root, signature, () => h("div.norepo", null,
    h("div.welcome.warn", { role: "status" }, icon("steered", 22), h("div", null,
      h("b", null, "No repo selected. Rerail’s dashboard shows one repo at a time."),
      h("p", null, "Open a coding agent in one of your repos and come here from there, or pick a repo below. A new repo is set up with ",
        h("span.mono", null, "/rerail-start"), " in a session there."))),
    h("section.group", null,
      h("div.ghead", null, h("h2", null, "Repos Rerail has seen"), h("p", null, "Pick one to see its rules and history.")),
      list.length === 0
        ? h("div.list", null, h("div.empty", null, h("h3", null, data.loaded ? "No repositories yet" : "Loading…"),
          h("p", null, "Run ", h("code", null, "rerail start"), " in a repository and it shows here.")))
        : h("div.list.repolist", { role: "listbox", "aria-label": "Repos Rerail has seen" }, list.map(repo => repoOption(repo, false))))));
}
