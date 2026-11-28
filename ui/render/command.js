/**
 * A command to run in a terminal, with a button that copies it (the common `cmdcopy` action). Create a rule and the
 * page that asks to connect this browser each show one. Also the pieces of a newer Rerail's notices (Overview's banner
 * and the top bar's popover): the update command for a coding agent's session, its Copy, and Details in Settings.
 */
import { h, icon } from "../dom.js";

export function commandBlock(command) {
  return h("div.cmd", null, h("code", null, command), h("button.btn.sm.solid", { "data-act": "cmdcopy", "data-i": command }, icon("copy"), "Copy"));
}

/** What updates Rerail, and its rule packs with it, pasted into a session with a coding agent. */
export const AGENT_UPDATE_COMMAND = "/rerail-update";

/** A button that copies `AGENT_UPDATE_COMMAND`; `spec` is its tag and classes. */
export const updateCopyButton = (spec, label) => h(spec, { "data-act": "cmdcopy", "data-i": AGENT_UPDATE_COMMAND, title: `${AGENT_UPDATE_COMMAND}, for your coding agent` },
  icon("copy"), label);

/** Details in Settings: About Rerail, with the release notes and how to update. */
export const updateDetailsLink = () => h("button.textlink", { "data-act": "nav", "data-i": "settings?section=about" }, "Details in Settings");
