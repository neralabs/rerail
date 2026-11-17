/**
 * Session titles (`data.titles`), from each session's first user prompts. The sessions list carries no titles, so
 * they load lazily per visible row, a few lookups at a time, and the page re-renders as each arrives (the listener
 * `poll.js` hands over when polling starts). A transcript already loaded from its start gives its title at once.
 */
import { get } from "./api.js";
import { data } from "./state.js";
import { promptText } from "../model/sessions-model.js";

const WEAK = /^(continue|yes|no|ok|okay|go|try again|thanks|thank you|do it|next|y|n)\.?$/i;
const TITLE_MAX = 96;
/**
 * First user prompt with three or more words that is not a bare acknowledgement; its first line, trimmed to a title.
 * Text the coding agent's harness wrote into a prompt (`promptText`) is never the title.
 */
export function titleFromPrompts(texts) {
  const prompts = texts.map(promptText).filter(Boolean);
  const t = prompts.find(u => u.split(/\s+/).length >= 3 && !WEAK.test(u.trim())) || prompts[0];
  if (!t) return null;
  const line = t.trim().split("\n")[0].replace(/\s+/g, " ").trim();
  return line.length > TITLE_MAX ? `${line.slice(0, TITLE_MAX - 1).trimEnd()}…` : line;
}

let onChange = () => {};
/** Re-render the page with `listener` once a title arrives; `startPolling` hands over the poll's own. */
export function setTitleListener(listener) {
  onChange = listener;
}

const titleQueue = [];
let titleWorkers = 0;
const TITLE_WORKERS = 4;
/** sessionId → `eventCount` the current or last lookup covered; a retry waits for new events. */
const titleTried = new Map();

function titleFromEvents(events) {
  return titleFromPrompts(events.filter(e => e.kind === "user-prompt").map(e => String(e.payload?.text ?? "")).filter(Boolean));
}

/** Whether a session looked up before (`tried`, the `eventCount` it covered) waits: it ended, or has no new events. */
const waitsForEvents = (session, tried) => tried !== undefined && (session.endedAt !== null || session.eventCount <= tried);

// Only a transcript loaded from its start holds the first prompts; a newer page would name a later one.
const loadedFromStart = detail => detail?.loaded && !detail.earlier && detail.events.length;

/**
 * Titles come from the first user prompts; sessions_list carries none, so they load lazily per visible
 * row. Cached transcripts resolve synchronously, so the caller's `titleOf` sees the result in the same
 * render. A session without a usable prompt stays "Untitled" until it records more events; it is never
 * re-fetched (or re-rendered) just because it rendered again.
 */
export function ensureTitle(session) {
  const id = session.sessionId;
  if (typeof data.titles.get(id) === "string") return;
  const tried = titleTried.get(id);
  if (waitsForEvents(session, tried)) return;
  titleTried.set(id, session.eventCount);
  const detail = data.sessionDetail.get(id);
  if (loadedFromStart(detail)) { data.titles.set(id, titleFromEvents(detail.events)); return; }
  if (!titleQueue.includes(id)) titleQueue.push(id);
  pumpTitles();
}

function pumpTitles() {
  while (titleWorkers < TITLE_WORKERS && titleQueue.length) {
    const id = titleQueue.shift();
    titleWorkers += 1;
    (async () => {
      try {
        const { body } = await get(`/api/sessions/${encodeURIComponent(id)}/events?after=0&limit=60`);
        const title = titleFromEvents(body.events);
        if (title !== null || data.titles.get(id) === undefined) data.titles.set(id, title);
        onChange();
      } catch { titleTried.delete(id); /* the row keeps "Untitled session" and retries on a later render */ }
      finally { titleWorkers -= 1; pumpTitles(); }
    })();
  }
}
