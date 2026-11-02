/**
 * How many rules each repository Rerail knows has, and how many of them run, read from that repository's own rules
 * (`GET /api/repository/rules?repository=<key>`): Settings' Repos rows and the repository choices. A repository set
 * up with nothing added has none, so the count, not `setUp`, says whether rules were added. Each read is conditional
 * on the ETag of the counts it holds, so a round where nothing changed costs one 304 per repository.
 */
import { get } from "./api.js";
import { toStoredRules } from "./poll.js";
import { isRunning } from "./derive.js";
import { fmt, plural } from "./format.js";

/** repositoryKey → `{ etag, counts: { total, on } }`. */
const known = new Map();
/** The last round queued; rounds run one after another so a later one never lands under an earlier one's answers. */
let lastRound = Promise.resolve(false);

/** `{ total, on }` of a repository's rule entries: one rule per id however many folders add it; on when it runs. */
export function ruleCountsOf(entries) {
  const rules = [...toStoredRules(entries).values()];
  return { total: rules.length, on: rules.filter(isRunning).length };
}

/** "12 rules on", "0 rules on" when every rule is off, "No rules added" when there are none; `null` while unknown. */
export function repoTag(counts) {
  if (counts === null) return null;
  return counts.total === 0 ? "No rules added" : `${fmt(counts.on)} ${plural(counts.on, "rule")} on`;
}

/** The words about a set-up repository's rules: its tag; "Off · 12 rules" while it is off with rules; On or Off until the counts are read. */
function rulesWords(started, counts) {
  if (counts === null) return [started ? "On" : "Off"];
  if (counts.total > 0 && !started) return ["Off", `${fmt(counts.total)} ${plural(counts.total, "rule")}`];
  return [repoTag(counts)];
}

/**
 * The line under a repository's name in the repository choices: "Set up · 12 rules on · 2 checkouts", "Set up · Off ·
 * 12 rules", "Set up · No rules added", "Not set up · nothing runs". One checkout goes unsaid.
 */
export function repoLine(repo, counts) {
  const checkouts = repo.checkoutCount > 1 ? [`${fmt(repo.checkoutCount)} checkouts`] : [];
  const state = repo.setUp ? ["Set up", ...rulesWords(repo.started, counts)] : ["Not set up", "nothing runs"];
  return [...state, ...checkouts].join(" · ");
}

/** The counts last read for repository `key`, or `null` before they are. */
export const repoRuleCounts = key => known.get(key)?.counts ?? null;

async function loadOne(key) {
  const held = known.get(key);
  try {
    const { status, etag, body } = await get(`/api/repository/rules?repository=${encodeURIComponent(key)}`, { etag: held?.etag });
    if (status === 304) return false;
    known.set(key, { etag, counts: ruleCountsOf(body.rules) });
    return true;
  } catch {
    // A repository that went away (404) or a failed read: no count rather than a stale one; the next round retries.
    return known.delete(key);
  }
}

async function readCounts(keys) {
  const listed = new Set(keys);
  const dropped = [...known.keys()].filter(key => !listed.has(key));
  for (const key of dropped) known.delete(key);
  const changed = await Promise.all([...listed].map(loadOne));
  return dropped.length > 0 || changed.some(Boolean);
}

/**
 * Read the counts of every repository in `keys`, dropping those of repositories no longer listed, once the round
 * before it is done. Resolves whether any count changed.
 */
export function loadRepoRuleCounts(keys) {
  const round = lastRound.then(() => readCounts(keys));
  lastRound = round.catch(() => false);
  return round;
}
