/**
 * What the Overview and the pages in its place show, as pure functions of what the poll loaded: the repository's
 * first-open state (which banner, which empty block), the tiles' figures and the Most violations logged ranking.
 * Nothing here reads `data` or `ui`; `views/overview.js` passes in what it needs.
 */

/**
 * @typedef {"no-repo" | "missing" | "not-set-up" | "stopped" | "no-packs" | "packs-no-session" | "mature"} FirstOpenState
 * @typedef {object} RepositoryFacts
 * @property {boolean} scoped         The URL names a repository or checkout.
 * @property {boolean} missing        The daemon does not know it.
 * @property {boolean} setUp          It has a configuration.
 * @property {boolean} stopped        Rerail is stopped where the page looks.
 * @property {number} ruleCount       Rules added to it, on or off (packs' and created by you).
 * @property {number} sessionsEver    Sessions recorded in it since install.
 */

/**
 * The repository's state, first match wins: no repository in the URL, one the daemon does not know, not set up,
 * stopped, no rule added (Jeff's decision 2: no banner, the empty block is the message), rules but no session yet
 * (the welcome banner), and sessions have run (no banner; the page is the answer).
 * @param {RepositoryFacts} facts
 * @returns {FirstOpenState}
 */
export function firstOpenState({ scoped, missing, setUp, stopped, ruleCount, sessionsEver }) {
  if (!scoped) return "no-repo";
  if (missing) return "missing";
  if (!setUp) return "not-set-up";
  if (stopped) return "stopped";
  if (ruleCount === 0) return "no-packs";
  if (sessionsEver === 0) return "packs-no-session";
  return "mature";
}

/**
 * The banner above the tiles: Start for a stopped repository, the welcome until it is dismissed for one whose rules
 * have not met a session yet, else none.
 * @param {FirstOpenState} state
 * @param {boolean} welcomeDismissed
 * @returns {"stopped" | "welcome" | null}
 */
export function bannerFor(state, welcomeDismissed) {
  if (state === "stopped") return "stopped";
  if (state === "packs-no-session" && !welcomeDismissed) return "welcome";
  return null;
}

/**
 * What the Overview shows under the tiles: the empty block for no rules, the one for rules that have met no session,
 * the one for a range without violations, or the two lists. A stopped repository shows what its data says.
 * @param {{ ruleCount: number, sessionsEver: number, topCount: number }} facts
 * @returns {"no-rules" | "no-sessions" | "no-violations" | "lists"}
 */
export function overviewBody({ ruleCount, sessionsEver, topCount }) {
  if (ruleCount === 0) return "no-rules";
  if (topCount > 0) return "lists";
  return sessionsEver === 0 ? "no-sessions" : "no-violations";
}

/**
 * Violations steered X of Y (Jeff's decision 1): Y every distinct violation in range, X those recorded while their
 * rule was on Steer or Block (distinct less the ones recorded in Log mode). `rerails` counts steer deliveries, which can
 * exceed Y, so it is not used. Unmeasured rules (conversation rules, inactive code rules) record none.
 * @param {Record<string, { measured?: boolean, distinctViolations: number, loggedViolations?: number }>} activity
 * @returns {{ steered: number, total: number }}
 */
export function steeredTally(activity) {
  let steered = 0, total = 0;
  for (const a of Object.values(activity)) {
    if (a.measured === false) continue;
    total += a.distinctViolations;
    steered += a.distinctViolations - (a.loggedViolations ?? 0);
  }
  return { steered, total };
}

/** How many rules Most violations logged lists. */
export const TOP_RULES = 10;

/**
 * The rules with violations in range, most first; ties by title, then id, so the order never shuffles between polls.
 * @template {{ id: string, title: string, fires: number }} R
 * @param {R[]} rules
 * @returns {R[]}
 */
export function topRules(rules, limit = TOP_RULES) {
  return rules.filter(r => r.fires > 0)
    .sort((a, b) => b.fires - a.fires || a.title.localeCompare(b.title) || a.id.localeCompare(b.id))
    .slice(0, limit);
}

/**
 * The Rules on tile's line: where the rules come from.
 * @param {number} packCount  Rule packs added here with at least one rule.
 * @param {number} mineCount  Rules created by you added here.
 */
export function rulesOnSource(packCount, mineCount) {
  if (packCount > 0) return `in ${packCount} rule ${packCount === 1 ? "pack" : "packs"}${mineCount > 0 ? " and yours" : ""}`;
  return mineCount > 0 ? "all created by you" : "no rules added yet";
}

/** Minutes an open session may stay silent and still read as in progress. */
export const LIVE_MINUTES = 30;

/**
 * An open session's state at `now`: "live" while heard from within {@link LIVE_MINUTES}, "stale" after (no end
 * recorded), `null` once it ended. Silence is measured from its newest event, else from its start.
 * @param {{ endedAt: number | null, startedAt: number, lastEventAt: number | null }} session
 * @param {number} now
 * @returns {"live" | "stale" | null}
 */
export function liveState(session, now) {
  if (session.endedAt !== null) return null;
  const last = session.lastEventAt ?? session.startedAt;
  return now - last <= LIVE_MINUTES * 60_000 ? "live" : "stale";
}
