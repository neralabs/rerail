/**
 * A command to run in a terminal, with a button that copies it (the common `cmdcopy` action). Create a rule and the
 * page that asks to connect this browser each show one.
 */
import { h, icon } from "../dom.js";

export function commandBlock(command) {
  return h("div.cmd", null, h("code", null, command), h("button.btn.sm.solid", { "data-act": "cmdcopy", "data-i": command }, icon("copy"), "Copy"));
}
