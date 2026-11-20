/**
 * A rule's enforcement, read-only: what Rerail does with it on a violation, as it runs now. The dashboard cannot change
 * it (a rule's mode is changed through the coding agent), so it is shown, not offered: a tag in the Enforcement column
 * of a rule table (a pack's page, Created by you, Overview) and on the "On a violation" line of the rule's sheet. The
 * tag takes the mode's own outcome colours and icon, as History shows them.
 */
import { h, icon, setAttr } from "../dom.js";
import { awaitingApproval, ruleMode } from "../derive.js";

/** Why the tag is not a control, in its hover and under it in the sheet. */
export const MODE_READ_ONLY = "Use Edit with coding agent to change this.";

/**
 * What a rule does as it runs: a rule that waits for approval runs in Log whatever its mode; Block steers until
 * blocking ships.
 */
export function enforcementOf(stored) {
  if (awaitingApproval(stored) || ruleMode(stored) === "log") {
    return { kind: "log", glyph: "logged", label: "Log only", tip: "Log only: Rerail records its violations; your coding agent isn’t told." };
  }
  return { kind: "steer", glyph: "steered", label: "Steer",
    tip: ruleMode(stored) === "block" ? "Set to Block, which steers for now." : "Steer: Rerail tells your coding agent as it works." };
}

/** An empty tag, filled by `updateEnforcementTag`. */
export const enforcementTag = () => h("span.enftag");

/** Bring a tag up to date with its rule. */
export function updateEnforcementTag(el, stored) {
  const e = enforcementOf(stored), key = `${e.kind}\0${e.tip}`;
  setAttr(el, "title", `${e.tip} ${MODE_READ_ONLY}`);
  if (el.dataset.key === key) return;
  el.dataset.key = key;
  setAttr(el, "class", `enftag ${e.kind}`);
  el.replaceChildren(icon(e.glyph, 12), e.label);
}

/** The column labels over a rule table: Rule · Violations · Enforcement · On/off. */
export const ruleTableHead = () => h("div.colhead.enfcols", { "aria-hidden": "true" },
  h("span", null, "Rule"), h("span", null, "Violations"), h("span", null, "Enforcement"), h("span", null, "On/off"));
