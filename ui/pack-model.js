/**
 * What the pack pages (`#pack/<id>`, `#created`), the catalog (`#packs`) and the pack sheet derive from the rules and
 * packs the page holds: a page's rules as filtered and sorted, the whole-pack switch's state, the catalog's kinds,
 * order and the packs suggested for the repository, and the sheet's rules. Nothing here touches the DOM.
 */
import { categoryLabel, isLog, isRunning, RISK_LABELS } from "./derive.js";
import { activityOf, dimensionsOf, fires, matches, RISK_FILTER, ruleSections, titleOf, ui } from "./state.js";

/** The Filter select of a pack page: `ui.filter` values `matches()` reads. */
export const PAGE_FILTERS = [["all", "All rules"], ["on", "On"], ["steer", "Steer"], ["log", "Log only"], ["off", "Off"], ["fired", "Has violations"]];
/**
 * The Filter select's risk choices for a page's `rules`: one `risk:<risk>` per risk their packs give at least one of
 * them, in the risks' order, with how many rules have it. None when no rule on the page has dimensions.
 */
export function pageRiskFilters(rules) {
  const counts = new Map();
  for (const r of rules) {
    const risk = dimensionsOf(r)?.risk;
    if (risk !== undefined) counts.set(risk, (counts.get(risk) ?? 0) + 1);
  }
  return RISK_LABELS.filter(([risk]) => counts.has(risk)).map(([risk, label]) => [`${RISK_FILTER}${risk}`, `${label} (${counts.get(risk)})`]);
}
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
 * The whole-pack switch over `rules`: how many are switched on, of how many, and its `aria-checked`: "true" when every
 * one is on, "mixed" when some are, "false" when none is (or there are none).
 */
export function packSwitch(rules) {
  const on = rules.filter(r => r.enabled).length, total = rules.length;
  return { on, total, state: total > 0 && on === total ? "true" : on > 0 ? "mixed" : "false" };
}

/** The counts line under a page's name: how many of `rules` steer, log only and are off ("6 steer · 15 log only"). */
export function enforcementCounts(rules) {
  const running = rules.filter(isRunning), log = running.filter(isLog).length, steer = running.length - log, off = rules.length - running.length;
  return [[steer, "steer"], [log, "log only"], [off, "off"]].filter(([n]) => n > 0).map(([n, word]) => `${n.toLocaleString("en-US")} ${word}`).join(" · ");
}

/** What a click on the whole-pack switch sets: off or mixed turns the rest on; on turns them all off. */
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

/**
 * Why Rerail suggested pack `packId` for this repository, from `GET /api/rule-packs/recommendations` (`""` when it gave
 * no reason), or `null` when it did not suggest it or the recommendations are not read yet.
 */
export function suggestedReason(recommendations, packId) {
  const pick = (recommendations ?? []).find(r => r.packId === packId);
  return pick === undefined ? null : pick.reason ?? "";
}

/**
 * The catalog's packs in two groups, the ones the recommendations suggest for this repository first, each group in
 * the order given; `null` while the recommendations are not read or suggest no pack, when the catalog is one list.
 */
export function suggestedSplit(packs, recommendations) {
  if (!recommendations?.length) return null;
  const picked = new Set(recommendations.map(r => r.packId));
  return { suggested: packs.filter(p => picked.has(p.id)), others: packs.filter(p => !picked.has(p.id)) };
}

/** A pack sheet gets a filter box once its pack has more rules than this. */
export const SHEET_FILTER_AT = 30;

/** A pack sheet's rows: the pack's rules whose title or id holds `query`, A to Z. */
export function sheetRules(pack, query) {
  const q = query.trim().toLowerCase();
  return pack.rules.filter(r => q === "" || `${r.title} ${r.id}`.toLowerCase().includes(q))
    .sort((a, b) => a.title.localeCompare(b.title));
}

/** A sheet's rows for a pack not added yet, by how each starts once it is: on, then off, each in the order given. */
export const startGroups = rows => ({ on: rows.filter(r => r.on), off: rows.filter(r => !r.on) });
