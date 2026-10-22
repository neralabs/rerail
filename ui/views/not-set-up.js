/**
 * The page a repository that is not set up shows in place of Overview and its pack pages: no rules run there yet;
 * the way to set it up, or to review its packs first.
 */
import { h, setContent } from "../dom.js";
import { ui, repoName } from "../state.js";
import { writes } from "../shell.js";

export const needs = [];

export function render(root) {
  const name = repoName();
  setContent(root, `not-set-up|${name}|${ui.settingUp}`, () => h("div.list.notsetup", null,
    h("div.empty", null, h("h3", null, `Rerail is not set up in ${name}`),
      h("p", null, "No rules run here yet. Run ", h("code", null, "rerail start"), " in the repository to choose its rule packs and start Rerail, or add the rule packs recommended for it now."),
      h("p.acts", null,
        h("button.btn.solid", { "data-act": "setup", disabled: ui.settingUp }, ui.settingUp ? "Setting up…" : "Set up with recommended packs"),
        " ",
        h("button.btn", { "data-act": "nav", "data-i": "packs?review=1" }, "Review rule packs")))));
}

export const actions = {
  setup: () => writes.setUpRecommended(),
};
