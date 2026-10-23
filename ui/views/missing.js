/**
 * The page for a repository or checkout in the URL that the daemon does not know (every view but Settings): what to
 * do there, and the way back to the repositories Rerail has seen.
 */
import { h, setContent } from "../dom.js";
import { ui } from "../state.js";
import { showRepository } from "../shell.js";

export const needs = [];

export function render(root) {
  const target = ui.scope.repository ?? ui.scope.checkout;
  setContent(root, `missing|${target}`, () => h("div.list.missing", null,
    h("div.empty", null, h("h3", null, "Rerail does not know this repo"),
      h("p", null, h("span.mono", null, target), ". Type ", h("span.mono", null, "/rerail-start"),
        " in a coding agent session there to set it up, or pick a repo Rerail has seen."),
      h("p.acts", null, h("button.btn.sm.solid", { "data-act": "pickother" }, "Pick another repo")))));
}

export const actions = {
  pickother: () => showRepository(null),
};
