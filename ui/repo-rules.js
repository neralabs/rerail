/**
 * How many rules each repository Rerail knows has, how many of them run, and how many wait for a person's approval,
 * read from that repository's own rules (`GET /api/repository/rules?repository=<key>`): Settings' Repos rows and the
 * repository choices. A repository set up with nothing added has none, so the count, not `setUp`, says whether rules
 * were added. Each read is conditional on the ETag of the counts it holds, so a round where nothing changed costs one
 * 304 per repository.
 */
import { get } from "./api.js";
import { toStoredRules } from "./poll.js";
import { isRunning } from "./derive.js";
import { fmt, plural } from "./format.js";

/** repositoryKey → `{ etag, counts: { total, on, awaiting } }`. */
const known = new Map();
/** The last round queued; rounds run one after another so a later one never lands under an earlier one's answers. */
let lastRound = Promise.resolve(false);

/**
 * `{ total, on, awaiting }` of a repository's rule entries: one rule per id however many folders add them; on when
 * it runs; `awaiting` the read's `awaitingApproval`, the rules whose content waits for a person's approval.
 */
export function ruleCountsOf(entries, awaitingApproval = 0) {
  const rules = [...toStoredRules(entries).values()];
  return { total: rules.length, on: rules.filter(isRunning).length, awaiting: awaitingApproval };
}

/** "2 waiting for approval" while rules wait, as one more word of the line; nothing while none do. */
const waitingWords = counts => (counts.awaiting > 0 ? [`${fmt(counts.awaiting)} waiting for approval`] : []);

/**
 * "12 rules on", "0 rules on" when every rule is off, "No rules added" when there are none, then how many wait for
 * approval while any do ("0 rules on · 2 waiting for approval"); `null` while unknown.
 */
export function repoTag(counts) {
  if (counts === null) return null;
  if (counts.total === 0 && counts.awaiting === 0) return "No rules added";
  return [`${fmt(counts.on)} ${plural(counts.on, "rule")} on`, ...waitingWords(counts)].join(" · ");
}

/** The words about a set-up repository's rules: its tag; "Off · 12 rules" while it is off with rules; On or Off until the counts are read. */
function rulesWords(started, counts) {
  if (counts === null) return [started ? "On" : "Off"];
  if (counts.total > 0 && !started) return ["Off", `${fmt(counts.total)} ${plural(counts.total, "rule")}`, ...waitingWords(counts)];
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
    known.set(key, { etag, counts: ruleCountsOf(body.rules, body.awaitingApproval) });
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
