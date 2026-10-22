/**
 * The controls over a rule list (Overview and the pack pages): search, the filter chips, Collapse all groups and the
 * sort. They set `ui.query`, `ui.filter` and `ui.sort`, which every rule list reads. A view that shows them spreads
 * `ruleControlActions` into its actions and passes its input and change events to `ruleControlInput`/`ruleControlChange`.
 */
import { h, setAttr, setText, searchIcon } from "../dom.js";
import { ui } from "../state.js";
import { invalidate } from "../shell.js";
import { allGroupsFolded, foldAll, sectionKeys } from "./rules.js";

const CHIPS = [["all", "All"], ["mine", "Created by you"], ["rerail", "By Rerail"], ["on", "On"], ["off", "Off"], ["fired", "Has violations"]];
const SORTS = [["violations", "Most violations logged"], ["recent", "Most recent violations logged"], ["az", "A to Z"]];

let built = null;

function build() {
  const search = h("input", { id: "q", type: "search", placeholder: "Search rules, ids or descriptions", "aria-label": "Search rules" });
  const chips = h("div.chips", null, CHIPS.map(([f, label]) => h("button.chip", { "data-act": "filter", "data-i": f, "aria-pressed": "false" }, label)));
  const fold = h("button.btn.sm.ghost.link", { "data-act": "foldall" });
  const sort = h("select", { id: "sort", "aria-label": "Sort" }, SORTS.map(([v, label]) => h("option", { value: v }, label)));
  const root = h("div.controls", null,
    h("div.crow", null, h("div.search", null, searchIcon(14), search)),
    h("div.crow", null, chips, h("span.grow"), fold, h("label.sortctl", null, h("span", null, "Sort"), sort)));
  built = { root, search, chips, fold, sort };
}

/** The controls' element, built once; `only` is the one section key the list shows, or `null` for every section. */
export function ruleControls(only = null) {
  if (built === null) build();
  for (const c of built.chips.children) setAttr(c, "aria-pressed", String(c.dataset.i === ui.filter));
  if (built.sort.value !== ui.sort) built.sort.value = ui.sort;
  if (built.search.value.trim().toLowerCase() !== ui.query && document.activeElement !== built.search) built.search.value = ui.query;
  setText(built.fold, allGroupsFolded(only) ? "Expand all groups" : "Collapse all groups");
  setAttr(built.fold, "hidden", sectionKeys(only).length === 0);
  setAttr(built.fold, "data-i", only ?? "");
  return built.root;
}

export const ruleControlActions = {
  filter: f => { ui.filter = f; invalidate(); },
  foldall: only => foldAll(only === "" ? null : only),
};

/** Input events on the page: the search box. Returns whether it was one of the controls'. */
export function ruleControlInput(el) {
  if (el.id !== "q") return false;
  ui.query = el.value.trim().toLowerCase();
  invalidate();
  return true;
}

/** Change events on the page: the sort. Returns whether it was one of the controls'. */
export function ruleControlChange(el) {
  if (el.id !== "sort") return false;
  ui.sort = el.value;
  invalidate();
  return true;
}
