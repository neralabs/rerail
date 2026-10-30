/**
 * History by rule: which rules it lists (under its All · Steered · Logged chips), which one it shows, and what it says
 * when there is nothing to list. `poll.js` reads `historyRuleId` to fetch that rule's violations in range.
 */
import { data, ui, sinceFor, titleOf, attachedPackOf, packOfRule } from "./state.js";
import { isByRerail } from "./derive.js";

/** History by rule's chips: the `ui.histOutcome` value and its label. */
export const OUTCOME_CHIPS = [["all", "All"], ["steered", "Steered"], ["logged", "Logged"]];

/**
 * Distinct violations of rule `id` in the range that `outcome` counts: every one ("all"), those recorded on Steer or
 * Block ("steered", as the Violations steered tile counts them), or those recorded on Log ("logged"). 0 for a rule the
 * range has no activity for, or one not measured.
 */
export function inRange(id, outcome = "all") {
  const a = data.activity[id];
  if (a === undefined || a.measured === false) return 0;
  const logged = a.loggedViolations ?? 0;
  return outcome === "steered" ? a.distinctViolations - logged : outcome === "logged" ? logged : a.distinctViolations;
}

/** A row's title: the stored rule's, else (a rule no longer here) its id. */
export const historyTitle = id => {
  const stored = data.rules.get(id);
  return stored ? titleOf(stored) : id;
};

/** Where a rule comes from, as its list row and heading read: its rule pack, else who made it. */
export function historyOrigin(id) {
  const stored = data.rules.get(id);
  if (!stored) return "No longer in this repo";
  if (!isByRerail(stored)) return "Created by you";
  return (attachedPackOf(stored) ?? packOfRule(stored))?.name ?? "By Rerail";
}

/** The listed rules the search box lets through: `query` (trimmed, any case) in the title, the id or the pack. */
export function filterRanked(ranked, query) {
  const q = query.trim().toLowerCase();
  if (q === "") return ranked;
  return ranked.filter(r => [r.title, r.id, r.origin].some(text => text.toLowerCase().includes(q)));
}

/**
 * The rules History lists: every rule with a violation in the range that `outcome` counts, most first, ties by title.
 * Rules no longer here still list (their violations stay in history) under their id.
 */
export function rankedRules(outcome = ui.histOutcome) {
  return Object.keys(data.activity)
    .filter(id => inRange(id, outcome) > 0)
    .map(id => ({ id, title: historyTitle(id), origin: historyOrigin(id), count: inRange(id, outcome) }))
    .sort((a, b) => b.count - a.count || a.title.localeCompare(b.title) || a.id.localeCompare(b.id));
}

/**
 * The rule History shows: the one chosen (a `?rule=` link, a click), whether or not it has violations in the range;
 * else the top of the list; `null` when the list is empty.
 */
export function historyRuleId(ranked = rankedRules()) {
  return ui.actRule ?? ranked[0]?.id ?? null;
}

/** Sessions since install, from whichever activity answer covers all time; `null` until it is in. */
function sessionsSinceInstall() {
  return sinceFor() === 0 ? data.activitySessions : data.activityAllSessions;
}

/**
 * What History shows when no rule has a violation in the range (whatever the chip), or `null` when some does:
 * - `noRules`: no rule is on, so nothing can be logged;
 * - `noSessions`: rules are on, but no coding agent session ran yet;
 * - `noViolations` (`sessions`: the audited sessions): sessions ran and no rule ever logged one;
 * - `nothingInRange`: violations exist, none in the range.
 */
export function historyEmpty(ranked = rankedRules("all")) {
  if (ranked.length > 0) return null;
  if (![...data.rules.values()].some(stored => stored.enabled)) return { kind: "noRules" };
  const sessions = sessionsSinceInstall()?.started ?? 0;
  if (sessions === 0) return { kind: "noSessions" };
  const everLogged = Object.values(sinceFor() === 0 ? data.activity : data.activityAll)
    .some(a => a.measured !== false && a.distinctViolations > 0);
  return everLogged ? { kind: "nothingInRange" } : { kind: "noViolations", sessions };
}
