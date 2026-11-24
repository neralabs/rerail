/**
 * The controls over a pack page's rule table: the search, then Filter and Sort with a slot at the right end of their
 * row (the page's whole-pack switch, on the On/off column). They set `ui.query`, `ui.filter` and `ui.sort`, which
 * `shownRules` (pack-model.js) reads. A view that shows them spreads `ruleControlActions` into its actions and passes
 * its input and change events to `ruleControlInput`/`ruleControlChange`.
 */
import { h, setAttr, setClass, setContent, searchIcon } from "../dom.js";
import { ui } from "../state.js";
import { invalidate } from "../shell.js";
import { PAGE_FILTERS, PAGE_SORTS } from "../pack-model.js";

let built = null;

function build() {
  const search = h("input", { id: "q", type: "search", placeholder: "Search rules, ids or descriptions", "aria-label": "Search rules" });
  const risks = h("optgroup", { label: "Risk", hidden: true });
  const filter = h("select", { id: "filter", "aria-label": "Filter rules" }, PAGE_FILTERS.map(([v, label]) => h("option", { value: v }, label)), risks);
  const filterCtl = h("label.sortctl", null, h("span", null, "Filter"), filter);
  const clear = h("button.btn.sm.ghost.link", { "data-act": "filterclear", title: "Show every rule" }, "Clear");
  const sort = h("select", { id: "sort", "aria-label": "Sort" }, PAGE_SORTS.map(([v, label]) => h("option", { value: v }, label)));
  const slot = h("span.ctlslot");
  const root = h("div.controls", null,
    h("div.crow", null, h("div.search", null, searchIcon(14), search)),
    h("div.crow", null, h("span.filterctl", null, filterCtl, clear), h("label.sortctl", null, h("span", null, "Sort"), sort), h("span.grow"), slot));
  built = { root, search, filter, risks, filterCtl, clear, sort, slot };
}

/**
 * The Filter select: the page's risk choices (`riskFilters`, `pageRiskFilters`) under Risk, which none hides; the
 * choice `ui.filter` names when the page offers it; and Clear while it narrows the list.
 */
function syncFilter(riskFilters) {
  setContent(built.risks, JSON.stringify(riskFilters), () => riskFilters.map(([v, label]) => h("option", { value: v }, label)));
  setAttr(built.risks, "hidden", riskFilters.length === 0);
  const offered = [...PAGE_FILTERS, ...riskFilters].some(([v]) => v === ui.filter);
  if (offered && built.filter.value !== ui.filter) built.filter.value = ui.filter;
  setClass(built.filterCtl, "on", ui.filter !== "all");
  setAttr(built.clear, "hidden", ui.filter === "all");
}

/**
 * The controls' element, built once, set to `ui`'s search, filter and sort; `slot` is the end of the Filter · Sort row.
 * `riskFilters` are the page's risk choices for the Filter (`syncFilter`).
 */
export function ruleControls(riskFilters) {
  if (built === null) build();
  syncFilter(riskFilters);
  if (built.sort.value !== ui.sort) built.sort.value = ui.sort;
  if (built.search.value.trim().toLowerCase() !== ui.query && document.activeElement !== built.search) built.search.value = ui.query;
  return { root: built.root, slot: built.slot };
}

export const ruleControlActions = {
  filterclear: () => { ui.filter = "all"; invalidate(); },
};

/** Input events on the page: the search box. Returns whether it was one of the controls'. */
export function ruleControlInput(el) {
  if (el.id !== "q") return false;
  ui.query = el.value.trim().toLowerCase();
  invalidate();
  return true;
}

/** Change events on the page: the filter and the sort. Returns whether it was one of the controls'. */
export function ruleControlChange(el) {
  if (el.id === "filter") ui.filter = el.value;
  else if (el.id === "sort") ui.sort = el.value;
  else return false;
  invalidate();
  return true;
}
