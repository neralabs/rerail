/**
 * Create a rule (`#create`): rules are written with the coding agent; the page shows the command to type.
 */
import { h, setContent, icon } from "../dom.js";
import { repoName } from "../state.js";
import { CREATE_COMMAND } from "../render/rules.js";

export const needs = [];

function commandBlock(command) {
  return h("div.cmd", null, h("code", null, command), h("button.btn.sm.solid", { "data-act": "cmdcopy", "data-i": command }, icon("copy"), "Copy"));
}

export function render(root) {
  const repo = repoName() || "this repo";
  setContent(root, `create|${repo}`, () => h("div.cpage.pbody", null,
    h("h1", null, "Create a rule"),
    h("p.pintro", null, "You create rules with your coding agent, in your own words. Type this in a session in ", h("span.mono", null, repo), ", a new one or one you already have open:"),
    commandBlock(CREATE_COMMAND),
    h("p.pintro", null, "It asks what the rule should look for, where it applies and what happens on a violation. Then it writes the rule and tells you its id."),
    h("p.pintro", null, "Your rule shows under Created by you in the menu, on by default, with every violation it logs or steers from the first session on.")));
}
