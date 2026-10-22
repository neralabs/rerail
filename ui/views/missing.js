/**
 * The page for a repository or checkout in the URL that the daemon does not know (every view but Settings).
 */
import { h, setContent } from "../dom.js";
import { ui } from "../state.js";

export const needs = [];

export function render(root) {
  const target = ui.scope.repository ?? ui.scope.checkout;
  setContent(root, `missing|${target}`, () => h("div.list.missing", null,
    h("div.empty", null, h("h3", null, "Rerail does not know this repository"),
      h("p", null, h("span.mono", null, target), ". Run ", h("code", null, "rerail start"), " in it, or choose another repository."))));
}
