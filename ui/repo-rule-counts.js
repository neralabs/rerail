/**
 * How many rules each repository Rerail knows has, and how many of them run, read from that repository's own rules
 * (`GET /api/repository/rules?repository=<key>`). Each read is conditional on the ETag of the counts it holds, so a round
 * where nothing changed costs one 304 per repository.
 */
import { get } from "./api.js";
import { ruleCountsOf } from "./repo-rules.js";

/** repositoryKey → `{ etag, counts: { total, on } }`. */
const known = new Map();
/** The last round queued; rounds run one after another so a later one never lands under an earlier one's answers. */
let lastRound = Promise.resolve(false);

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
