/**
 * The long lists the page pages through with a cursor (`pagedList` in `state.js`): History's and a rule sheet's
 * violations, the sessions list and a session's violations. A poll refreshes a list's first page and keeps the
 * further pages the person loaded, as long as the fresh page still reaches them; Load more appends the next page.
 */
import { get } from "./api.js";
import { data, pagedList } from "./state.js";
import { fetchFresh, generation, refetch } from "./scoped-requests.js";

/** How a row of each paged resource is told apart from the others. */
const ROW_ID = { violations: r => `${r.auditId}:${r.id}`, sessions: r => r.sessionId };

/**
 * The first page of the paged list `data[name]` at `key` (its URL without a cursor; `field` names the rows in the
 * answer), conditionally. Another key starts the list over at once, dropping the previous key's pages. Under the
 * same key, pages loaded beyond the first stay, deduplicated, with the cursor past the last of them, as long as the
 * fresh page reaches them: when more rows arrived than a page holds, the rows between the fresh page and the kept
 * ones could never be loaded, so the list starts over from the fresh page and its own cursor.
 */
export async function loadFirstPage(name, field, key, owner = null) {
  if (data[name].key !== key) {
    refetch(name);
    data[name] = pagedList(key, owner);
  }
  const body = await fetchFresh(key, name, () => data[name].key === key);
  if (body === undefined) return false;
  data[name] = withFirstPage(data[name], body, field);
  return true;
}

/** Paged list `current` with the fresh first page `body` (its rows in `body[field]`) in. */
function withFirstPage(current, body, field) {
  const rows = body[field];
  const id = ROW_ID[field];
  const kept = new Set(current.rows.map(id));
  // The fresh page is the whole list, or it does not reach the kept pages: it replaces them.
  const complete = body.nextCursor === null;
  const reaches = rows.length > 0 && kept.has(id(rows.at(-1)));
  if (current.pages <= 1 || complete || !reaches) return { ...current, rows, total: body.total, next: body.nextCursor, pages: 1 };
  const fresh = new Set(rows.map(id));
  return { ...current, rows: rows.concat(current.rows.filter(r => !fresh.has(id(r)))), total: body.total };
}

/**
 * The page after `data[name]`'s cursor, appended without duplicates. `false` when there is none, or when the list
 * moved on meanwhile (another key or scope, or this page already appended by an earlier click).
 */
export async function loadNextPage(name, field) {
  const { key, next } = data[name];
  if (key === null || next === null) return false;
  const started = generation;
  const { body } = await get(`${key}&cursor=${encodeURIComponent(next)}`);
  const current = data[name];
  if (started !== generation || current.key !== key || current.next !== next) return false;
  const id = ROW_ID[field], have = new Set(current.rows.map(id));
  data[name] = {
    ...current, rows: current.rows.concat(body[field].filter(r => !have.has(id(r)))),
    total: body.total, next: body.nextCursor, pages: current.pages + 1,
  };
  return true;
}

/** History: the next page of the shown rule's violations. */
export const loadMoreFindings = () => loadNextPage("findings", "violations");

/** Sessions view: the next page of the list. */
export const loadMoreSessions = () => loadNextPage("sessions", "sessions");
