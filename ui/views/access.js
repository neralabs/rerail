/**
 * The page a browser without the daemon's access capability shows in place of every view, Settings included: the
 * daemon answers `401` to every read (`data.accessRequired`), so no view has anything true to show. It asks to
 * connect this browser and names the command that does, `rerail manage`. Its CSS hides the rest of the frame, whose
 * repository, status and sidebar would otherwise read as an empty install. The poll goes on: once `rerail manage`
 * stores the capability in this browser, the next read carries it and the view the URL names comes back.
 */
import { h, setContent } from "../dom.js";
import { commandBlock } from "../render/command.js";

export const needs = [];

export function render(root) {
  setContent(root, "access", () => h("div.list.access", null,
    h("div.empty", { role: "status" },
      h("h3", null, "Connect this browser to Rerail"),
      h("p", null, "Rerail shows this computer’s rules and sessions only in a browser you’ve connected."),
      h("p", null, "Run this in a terminal on this computer. It connects your default browser and prints a link that connects any other. Once this browser is connected, this page continues on its own."),
      commandBlock("rerail manage"))));
}
