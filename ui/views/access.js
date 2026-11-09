/**
 * The page a browser without the daemon's access capability shows in place of every view, Settings included: the
 * daemon answers `401` to every read (`data.accessRequired`), so no view has anything true to show. It says why (a
 * link copied from a coding agent's session, or from another browser's address bar, leaves the capability out) and
 * names the command that gives this browser access.
 */
import { h, setContent, icon } from "../dom.js";

export const needs = [];

const COMMAND = "rerail manage";

export function render(root) {
  setContent(root, "access", () => h("div.list.access", null,
    h("div.empty", { role: "status" },
      h("h3", null, "This browser doesn’t have access to Rerail’s data yet."),
      h("p", null, "Run ", h("span.mono", null, COMMAND), " in a terminal on this computer. It opens the dashboard with access, and prints a link that gives access to any other browser you open it in."),
      h("p", null, "A link copied from a coding agent’s session, or from another browser’s address bar, leaves the access out."),
      h("p.acts", null, h("button.btn.sm.solid", { "data-act": "cmdcopy", "data-i": COMMAND }, icon("copy"), `Copy ${COMMAND}`)))));
}
