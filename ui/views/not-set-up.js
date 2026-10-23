/**
 * The page a repository that is not set up shows in place of Overview and its pack pages: nothing runs there yet.
 * It offers the set-up in the page, rather than sending people back to a terminal: the rule packs recommended for it in one click, the review of them
 * first, or the catalog; `/rerail-start` in a coding agent session does the same from there.
 */
import { h, setContent, icon } from "../dom.js";
import { ui, repoName } from "../state.js";
import { writes } from "../shell.js";

export const needs = [];

export function render(root) {
  const name = repoName();
  setContent(root, `not-set-up|${name}|${ui.settingUp}`, () => h("div.notsetup", null,
    h("div.welcome.warn", { role: "status" }, icon("steered", 22), h("div", null,
      h("b", null, `${name} is not set up. Nothing runs here.`),
      h("p", null, "No rule packs have been chosen for it. Set it up here, or from a coding agent session in this repo: type ",
        h("span.mono", null, "/rerail-start"), " there. It reads the repo, detects which rule packs apply and shows them for you to confirm."))),
    h("section.group", null,
      h("div.ghead", null, h("h2", null, "Rule packs ", h("span.n2", null, "0")),
        h("p", null, "Nothing runs here until rule packs are added.")),
      h("div.list", null, h("div.empty", null,
        h("h3", null, "No rule packs are on"),
        h("p", null, "Add the rule packs Rerail recommends for this repo in one go, review them first, or pick rule packs by hand."),
        h("p.acts", null,
          h("button.btn.sm.solid", { "data-act": "setup", disabled: ui.settingUp }, ui.settingUp ? "Setting up…" : "Set up with recommended packs"),
          h("button.btn.sm", { "data-act": "nav", "data-i": "packs?review=1" }, "Review rule packs"),
          h("button.btn.sm.ghost.link", { "data-act": "nav", "data-i": "packs" }, icon("plus"), "Add rule packs")))))));
}

export const actions = {
  setup: () => writes.setUpRecommended(),
};
