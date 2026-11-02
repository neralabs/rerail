/**
 * What the pack pages (`#pack/<id>`, `#created`), the catalog (`#packs`) and the pack sheet derive from the rules and
 * packs the page holds: a page's rules as filtered and sorted, the whole-pack switch's state, and the catalog's kinds
 * and order. Nothing here touches the DOM.
 */
import { categoryLabel, isRunning } from "./derive.js";
import { activityOf, fires, matches, ruleSections, titleOf, ui } from "./state.js";

/** The Filter select of a pack page: `ui.filter` values `matches()` reads. */
export const PAGE_FILTERS = [["all", "All rules"], ["on", "On"], ["off", "Off"], ["fired", "Has violations"]];
/** The Sort select of a pack page: `ui.sort` values `sortPageRules` reads. */
export const PAGE_SORTS = [["violations", "Most violations logged"], ["recent", "Most recent violations logged"], ["newest", "Newest first"], ["az", "A to Z"]];

/** The rule list section a pack page shows: Created by you, or the pack's. */
export const pageKey = (view, packId) => (view === "created" ? "mine" : `pack:${packId}`);

/** Every rule of a page's section (`pageKey`); `null` when the section is not here (the pack is not added). */
export function pageRules(key) {
  const section = ruleSections().find(s => s.key === key);
  return section === undefined ? null : section.groups.flatMap(g => g.all);
}

const byTitle = (a, b) => titleOf(a).localeCompare(titleOf(b));

/**
 * `rules` in the order `sort` names: most violations logged in range, the most recent violation first, the rule added
 * here most recently first (rules without a date last), or A to Z. Ties go A to Z.
 */
export function sortPageRules(rules, sort) {
  const c = [...rules];
  switch (sort) {
    case "az": return c.sort(byTitle);
    case "recent": return c.sort((a, b) => (activityOf(b.rule.id).lastSeenAt ?? 0) - (activityOf(a.rule.id).lastSeenAt ?? 0) || byTitle(a, b));
    case "newest": return c.sort((a, b) => (b.addedAt ?? -Infinity) - (a.addedAt ?? -Infinity) || byTitle(a, b));
    default: return c.sort((a, b) => fires(b.rule.id) - fires(a.rule.id) || byTitle(a, b));
  }
}

/** The rows a page shows: its rules the search and Filter let through, in the Sort's order. */
export const shownRules = rules => sortPageRules(rules.filter(matches), ui.sort);

/** How many of `rules` run (switched on, with options that apply): the page's "x of y on", as the sidebar counts. */
export const runningOf = rules => rules.filter(isRunning).length;

/**
 * The whole-pack switch over `rules`: how many are switched on, of how many, and its `aria-checked`. Binary: "true"
 * only when every one is on (and there is one), else "false"; the on-count chip says how many are on.
 */
export function packSwitch(rules) {
  const on = rules.filter(r => r.enabled).length, total = rules.length;
  return { on, total, state: total > 0 && on === total ? "true" : "false" };
}

/** What a click on the whole-pack switch sets: off turns the rest on; on turns them all off. */
export const switchTurnsOn = state => state !== "true";

/* ---------- the catalog ---------- */

/** The catalog's kind chips: All, then each category the packs have, languages first, then frameworks. */
const KIND_ORDER = ["language", "framework", "repository"];
export function catalogKinds(packs) {
  const present = [...new Set(packs.map(p => p.category))]
    .sort((a, b) => (KIND_ORDER.indexOf(a) + 1 || 99) - (KIND_ORDER.indexOf(b) + 1 || 99) || a.localeCompare(b));
  return [["all", "All"], ...present.map(c => [c, categoryLabel(c) || c])];
}

/** The packs of kind `kind` ("all": every one), alphabetical by name. */
export function catalogPacks(packs, kind) {
  return packs.filter(p => kind === "all" || p.category === kind)
    .sort((a, b) => a.name.localeCompare(b.name, undefined, { sensitivity: "base" }));
}

/** A pack sheet gets a filter box once its pack has more rules than this. */
export const SHEET_FILTER_AT = 30;

/** A pack sheet's rows: the pack's rules whose title or id holds `query`, A to Z. */
export function sheetRules(pack, query) {
  const q = query.trim().toLowerCase();
  return pack.rules.filter(r => q === "" || `${r.title} ${r.id}`.toLowerCase().includes(q))
    .sort((a, b) => a.title.localeCompare(b.title));
}
