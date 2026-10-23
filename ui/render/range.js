/**
 * The time range control. Overview and History each carry one; both set `ui.range` through the common `range`
 * action (`actions.js`).
 */
import { h, setAttr } from "../dom.js";
import { RANGES, ui } from "../state.js";

export function rangeControl() {
  return h("div.rangectl", { role: "group", "aria-label": "Time range" },
    RANGES.map(([d, l]) => h("button", { "data-act": "range", "data-i": d, "aria-pressed": "false" }, l)));
}

export function syncRange(control) {
  for (const b of control.children) setAttr(b, "aria-pressed", String(Number(b.dataset.i) === ui.range));
}
