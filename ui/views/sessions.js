/**
 * History, by session (`#sessions`, `?sessionId=<id>&host=<agent>` selects one): the repository's coding agent
 * sessions by day, and the selected one's transcript with the violations Rerail logged and steered in it.
 */
import { h, setAttr, setText, setClass, chevron, searchIcon, syncChildren } from "../dom.js";
import { fmt, plural, ago, duration, stamp, dayLabel, daysAgo, shortSessionId } from "../format.js";
import { data, ui } from "../state.js";
import { ensureTitle, loadSession, loadSessionFindings, loadMoreSessions, loadEarlier } from "../poll.js";
import { sessionTurns, sessionCorrections, sessionViolationTotal, loadedFrom, placeCorrections, resumeCommand, openState, lastEventAt, isOpen, violationCounts } from "../sessions-model.js";
import { copyText, invalidate, loadMore, renderNow, toast } from "../shell.js";
import { renderTimeline, jumpTo, placeTicks } from "../render/timeline.js";

export const needs = ["rules", "sessions", "selectedSession"];

/* ---------- selection ---------- */

/** The load behind the current selection, while it runs; a fallback must not pre-empt a deep link's 404 check. */
let selecting = null;

/**
 * Select a session and load it: its head and first transcript page, and its violations (the previous selection's are
 * dropped). Resolves to the transcript's load error (`null` on success) so a caller can tell a missing session from a slow one.
 */
async function selectSession(id, then) {
  ui.selectedSession = id;
  ui.resumeOpen = false;
  invalidate();
  if (id === null) return null;
  const violations = loadSessionFindings(id).catch(e => console.warn("session violations failed", e));
  const load = loadSession(id).then(() => null, e => e);
  selecting = load;
  const [error] = await Promise.all([load, violations]);
  if (selecting === load) selecting = null;
  if (error) console.warn("session load failed", error);
  renderNow();
  if (then) then();
  return error;
}

/** Scroll the transcript to its first violation. */
function jumpToFirst() {
  const first = document.querySelector(".tl .corr");
  if (first) jumpTo(first.dataset.key);
}

/** Select session `id` from a link; the default one, with a note, when the daemon has no such session. */
async function selectLinked(id, jump) {
  const error = await selectSession(id, jump ? jumpToFirst : undefined);
  if (error === null || !error.message.endsWith("answered 404")) return;
  if (ui.selectedSession === id) selectSession(currentSession()?.sessionId ?? null);
  toast(`Session ${shortSessionId(id, new URLSearchParams(location.search).get("host"))} is not in this repo`);
}

/** Entered with a session (a link, a violation's session): that one; else the one the view would show anyway. */
export function enter(route) {
  if (route.session !== undefined) selectLinked(route.session, route.jump === true);
  else selectSession(ui.selectedSession ?? currentSession()?.sessionId ?? null);
}

/** No selection, or one the list no longer has: pick what the view would show anyway and load it now, as a click would. */
export function afterPoll() {
  if (selecting !== null) return;
  const sess = currentSession();
  if (sess !== null && sess.sessionId !== ui.selectedSession) selectSession(sess.sessionId);
}

/* ---------- the view ---------- */

let v = null;
/** Session whose day group was last revealed; a new selection opens its day once, then the user owns the fold. */
let revealed = null;
/** The session list is shown by default; hiding it lasts for the visit. */
let listHidden = false;

function dayKey(d) {
  if (d < 7) return `d${d}`;
  if (d < 30) return `w${Math.floor(d / 7)}`;
  return "older";
}
const titleOf = id => data.titles.get(id) ?? "Untitled session";

function build() {
  const el = {};
  el.pick = h("select", { id: "spick", "aria-label": "Choose a session" });
  el.lead = h("p");
  el.search = h("input", { id: "sq", type: "search", placeholder: "Search title or id", "aria-label": "Search sessions" });
  el.chips = h("div.chips", { style: "margin-bottom:8px" },
    h("button.chip", { "data-act": "sfilter", "data-i": "all" }, "All"),
    h("button.chip", { "data-act": "sfilter", "data-i": "corr" }, "With violations"),
    h("button.chip", { "data-act": "sfilter", "data-i": "open" }, "In progress"));
  el.list = h("div.srows");
  el.listBox = h("div.list.slist", null, h("div.slhead", null, h("span", null, "Session"), h("span", null, "Violations logged")), el.list);
  el.listEmpty = h("div.empty", { hidden: true }, h("p", null, "No sessions match."));
  el.more = h("div.smore", { hidden: true }, h("button.btn.sm", { "data-act": "sessmore" }));
  el.title = h("h2.clamp2");
  el.hpick = h("select", { id: "hpick", "aria-label": "Choose a session" });
  el.hpickLabel = h("span.m", { "aria-hidden": "true" });
  el.hpickWrap = h("h2.hpick", { hidden: true }, h("span.sel", null, el.hpickLabel, el.hpick, chevron(12)));
  el.state = h("span", { hidden: true });
  el.started = h("span"); el.dur = h("span"); el.events = h("span"); el.sid = h("span.mono");
  el.copyId = h("button.btn.sm", { "data-act": "copyid" }, "Copy id");
  el.resume = h("button.btn.sm.solid", { "data-act": "resumetoggle", "aria-expanded": "false" });
  el.resumeLbl = h("p.lbl");
  el.resumeText = h("textarea.one", { id: "resumeText", readonly: true, "aria-label": "Resume command" });
  el.resumeHow = h("p", { style: "margin:0;font-size:13px", hidden: true });
  el.resumeFoot = h("span");
  el.resumeCopy = h("button.btn.sm.solid", { "data-act": "resumecopy" });
  el.resumeBox = h("div.pask.sresume", { hidden: true },
    h("button.x", { "data-act": "resumetoggle", "aria-label": "Close", title: "Close" }, "✕"),
    el.resumeLbl, el.resumeText, el.resumeHow, h("div.foot", null, el.resumeFoot, el.resumeCopy));
  el.tsum = h("span.tsum");
  el.tchips = h("span.tchips", { role: "group", "aria-label": "Event type", hidden: true },
    [["all", "All"], ["changes", "Changes"], ["commands", "Commands"], ["outside", "Outside & asks"], ["failures", "Failures"]].map(([k, l]) =>
      h("button.chip", { "data-act": "evfilter", "data-i": k, "aria-pressed": "false" }, l, k === "all" ? null : h("span.n"))));
  el.showAll = h("input", { type: "checkbox", id: "showAll" });
  el.rail = h("div.rail", { id: "rail" }, h("div.line"));
  el.tl = h("div.tl", { id: "tl" });
  el.scroll = h("div.scroll", { id: "sscroll" }, h("div.tlwrap", null, el.rail, el.tl));
  el.empty = h("div.list", { hidden: true }, h("div.empty", null, h("h3", null, "No sessions yet"), h("p", null, "Your first coding agent session in this repo shows here, with the violations it logged.")));
  el.listToggle = h("button.ltoggle", { "data-act": "slisttoggle", "aria-pressed": "true" });
  el.ltrow = h("div.ltrow", null, el.listToggle);
  el.aside = h("aside", null,
    h("div.spick", null, h("label", { for: "spick" }, "Session"), el.pick),
    h("div.ssearch", null, searchIcon(13), el.search),
    el.chips, el.listBox, el.listEmpty, el.more);
  el.sess = h("div.sess", null,
    el.aside,
    h("section", null, h("div.list", null,
      h("div.shead", null,
        h("div.l1", null, el.title, el.hpickWrap, el.state),
        h("div.l3", null, el.started, el.dur, el.events, el.sid, h("span.acts", null, el.copyId, el.resume)),
        el.resumeBox),
      h("div.tfilter", null, el.tsum, el.tchips, h("label.tall", null, el.showAll, " Show all events")),
      el.scroll)));
  el.root = h("div", null, h("div.ghead.hhead", null, h("h2", null, "History by session"), el.lead), el.ltrow, el.empty, el.sess);
  v = el;
}

function createListItem(item) {
  if (item.group) {
    return h("button.daygrp", { "data-act": "daytoggle", "data-i": item.dayKey, "aria-expanded": "false" }, chevron(), h("span.name"), h("span.cnt"));
  }
  const s = item.s;
  return h("button.srow", { "data-act": "ssel", "data-i": s.sessionId, "aria-current": "false" },
    h("span.l", null, h("span.t"), h("span.m", null, h("span.state"), h("span.ev"), h("span.sid"))),
    h("span.c.num", null, h("b"), h("span.st")));
}

function updateListItem(node, item) {
  if (item.group) {
    setAttr(node, "aria-expanded", String(item.open));
    setText(node.querySelector(".name"), item.name);
    const withC = item.withC ? ` · ${fmt(item.withC)} with violations` : "";
    setText(node.querySelector(".cnt"), `${fmt(item.count)} ${plural(item.count, "session")}${withC}`);
    return;
  }
  const s = item.s;
  ensureTitle(s);
  const title = titleOf(s.sessionId);
  setAttr(node, "aria-current", String(s.sessionId === ui.selectedSession));
  const t = node.querySelector(".t");
  setText(t, title); setAttr(t, "title", title);
  const st = openState(s), state = node.querySelector(".state");
  setAttr(state, "class", `state ${st ?? ""}`);
  setText(state, rowState(st, s));
  setAttr(state, "title", "Last event");
  setText(node.querySelector(".ev"), `${fmt(s.eventCount)} events`);
  setText(node.querySelector(".sid"), shortSessionId(s.sessionId, s.host));
  const c = node.querySelector(".c"), { logged, steered } = violationCounts(s);
  setClass(c, "zero", logged === 0);
  setText(c.firstChild, fmt(logged));
  setText(c.lastChild, logged === 0 ? "" : `${fmt(steered)} steered`);
  setAttr(c, "title", `${fmt(logged)} ${plural(logged, "violation")} logged, ${fmt(steered)} steered`);
}

function rowState(st, s) {
  if (st === "live") return `in progress · ${ago(lastEventAt(s))}`;
  if (st === "stale") return `no end recorded · ${ago(lastEventAt(s))}`;
  return ago(lastEventAt(s));
}

/**
 * The session the right column shows: the selected one (from the loaded list, else its own head, so a deep-linked
 * session past the loaded pages still shows), else the newest. `afterPoll` selects (and loads) the fallback.
 */
function currentSession() {
  const rows = data.sessions.rows, id = ui.selectedSession;
  return rows.find(s => s.sessionId === id) ?? (id === null ? null : data.sessionDetail.get(id)?.head) ?? rows[0] ?? null;
}

export function render(root) {
  if (v === null) build();
  if (v.root.parentNode !== root) root.replaceChildren(v.root);
  const all = data.sessions.rows;
  const sess = currentSession();
  setAttr(v.empty, "hidden", sess !== null);
  setAttr(v.sess, "hidden", sess === null);
  setAttr(v.ltrow, "hidden", sess === null);
  setAttr(v.lead, "hidden", sess === null);
  if (sess === null) return;
  renderListToggle();
  renderList(all, sess);
  renderHeader(sess);
  renderTranscript(sess);
}

function passesFilter(s) {
  if (ui.sessFilter === "corr") return s.violationCount > 0;
  if (ui.sessFilter === "open") return isOpen(s);
  return ui.sessFilter === "all";
}

const matchesQuery = (s, q) => !q || titleOf(s.sessionId).toLowerCase().includes(q) || shortSessionId(s.sessionId, s.host).includes(q) || s.sessionId.includes(q);

/** Consecutive runs of sessions (already newest first) that started on the same day bucket. */
function groupByDay(sessions) {
  const groups = [];
  for (const s of sessions) {
    const d = daysAgo(s.startedAt), key = dayKey(d);
    let g = groups.at(-1);
    if (!g || g.key !== key) { g = { key, name: dayLabel(d), rows: [] }; groups.push(g); }
    g.rows.push(s);
  }
  return groups;
}

/** The list items one day group contributes: its header, then (when open) its loaded rows. */
function groupItems(g, forceOpen) {
  const open = forceOpen || ui.openDays.has(g.key);
  const items = [{ key: `g:${g.key}`, dayKey: g.key, group: true, open, name: g.name, count: g.rows.length, withC: g.rows.filter(s => s.violationCount > 0).length }];
  if (open) for (const s of g.rows) items.push({ key: s.sessionId, s });
  return items;
}

function renderList(all, sess) {
  const q = ui.sessQuery.trim().toLowerCase();
  const visible = all.filter(s => passesFilter(s) && matchesQuery(s, q));
  const total = Math.max(data.sessions.total, all.length);
  // Filters and search run over the loaded pages; the total says how many more Load more can bring.
  setText(v.lead, `${fmt(visible.length)} of ${fmt(total)} · newest first`);
  if (v.search.value !== ui.sessQuery && document.activeElement !== v.search) v.search.value = ui.sessQuery;
  for (const c of v.chips.children) setAttr(c, "aria-pressed", String(c.dataset.i === ui.sessFilter));

  if (sess.sessionId !== revealed) { revealed = sess.sessionId; ui.openDays.add(dayKey(daysAgo(sess.startedAt))); }
  const items = groupByDay(visible).flatMap(g => groupItems(g, q !== ""));
  syncChildren(v.list, items, i => i.key, createListItem, updateListItem);
  setAttr(v.listEmpty, "hidden", items.length > 0);
  const more = data.sessions.next !== null;
  setAttr(v.more, "hidden", !more);
  if (more) setText(v.more.firstChild, `Load more · ${fmt(all.length)} of ${fmt(total)} loaded`);
  syncChildren(v.pick, visible.slice(0, 200), s => s.sessionId, s => h("option", { value: s.sessionId }),
    (o, s) => { setText(o, `${titleOf(s.sessionId)} · ${shortSessionId(s.sessionId, s.host)}`); });
  if (v.pick.value !== sess.sessionId) v.pick.value = sess.sessionId;
  if (listHidden) renderHeadPicker(all, sess);
}

/** List hidden: the session heading is the picker, over every loaded session (and the selected one, if past them). */
function renderHeadPicker(all, sess) {
  const rows = all.some(s => s.sessionId === sess.sessionId) ? all : [sess, ...all];
  syncChildren(v.hpick, rows, s => s.sessionId, s => h("option", { value: s.sessionId }), (o, s) => { setText(o, titleOf(s.sessionId)); });
  if (v.hpick.value !== sess.sessionId) v.hpick.value = sess.sessionId;
  const title = titleOf(sess.sessionId);
  setText(v.hpickLabel, title); setAttr(v.hpick, "title", title);
}

function renderListToggle() {
  setClass(v.sess, "nolist", listHidden);
  setAttr(v.aside, "hidden", listHidden);
  setAttr(v.title, "hidden", listHidden);
  setAttr(v.hpickWrap, "hidden", !listHidden);
  setAttr(v.listToggle, "aria-pressed", String(!listHidden));
  const label = listHidden ? "Show the session list" : "Hide the session list";
  setAttr(v.listToggle, "title", label); setAttr(v.listToggle, "aria-label", label);
  setText(v.listToggle, listHidden ? "Show list" : "Hide list");
}

function headerState(st, sess) {
  if (st === "live") return `In progress · last event ${ago(lastEventAt(sess))}`;
  if (st === "stale") return `No end recorded · last event ${ago(lastEventAt(sess))}`;
  return "";
}

function resumeLabel(rc, st) {
  if (!rc.cmd) return `Find in ${rc.name}`;
  return st === "live" ? "Continue session" : "Resume session";
}

function renderHeader(sess) {
  ensureTitle(sess);
  const title = titleOf(sess.sessionId);
  setText(v.title, title); setAttr(v.title, "title", title);
  const st = openState(sess);
  setAttr(v.state, "hidden", st === null);
  setAttr(v.state, "class", st ?? "");
  setText(v.state, headerState(st, sess));
  setText(v.started, `Started ${stamp(sess.startedAt)}`);
  setText(v.dur, duration((sess.endedAt ?? lastEventAt(sess)) - sess.startedAt));
  setText(v.events, `${fmt(sess.eventCount)} events`);
  setText(v.sid, shortSessionId(sess.sessionId, sess.host));
  setAttr(v.copyId, "data-i", sess.sessionId); setAttr(v.copyId, "title", sess.sessionId);
  const rc = resumeCommand(sess);
  setText(v.resume, resumeLabel(rc, st));
  setAttr(v.resume, "aria-expanded", String(ui.resumeOpen));
  setAttr(v.resumeBox, "hidden", !ui.resumeOpen);
  if (ui.resumeOpen) renderResumeBox(rc);
}

function renderResumeBox(rc) {
  setText(v.resumeLbl, rc.cmd ? `Run this in your terminal to pick the conversation back up in ${rc.name}` : `${rc.name} has no resume command`);
  setAttr(v.resumeText, "hidden", !rc.cmd);
  if (rc.cmd && v.resumeText.value !== rc.cmd) v.resumeText.value = rc.cmd;
  setAttr(v.resumeHow, "hidden", Boolean(rc.cmd)); setText(v.resumeHow, rc.how ?? "");
  setText(v.resumeFoot, rc.cmd ? "Reopens the same conversation with its history." : "The id is what to look for.");
  setText(v.resumeCopy, rc.cmd ? "Copy command" : "Copy id");
}

function updateEventChip(c, count) {
  const k = c.dataset.i;
  setAttr(c, "aria-pressed", String(ui.evFilter === k));
  setAttr(c, "disabled", count === 0 && ui.evFilter !== k ? true : null);
  setClass(c, "bad", k === "failures" && count > 0);
  if (count !== undefined) setText(c.querySelector(".n"), fmt(count));
}

function renderTranscript(sess) {
  const detail = data.sessionDetail.get(sess.sessionId);
  const from = loadedFrom(sess.sessionId);
  // Only as far back as the violations are loaded too, so no turn reads as clean for want of a page.
  const events = (detail?.events ?? []).filter(e => e.observedAt >= from);
  const cs = sessionCorrections(sess.sessionId);
  const total = sessionViolationTotal(sess.sessionId);
  const logged = total ?? cs.length;
  const turns = placeCorrections(sessionTurns(events), cs);
  const tools = turns.flatMap(t => t.tools);
  const n = lvl => tools.filter(e => e.level === lvl).length;
  setText(v.tsum, "");
  v.tsum.replaceChildren(h("b", null, fmt(ui.showAll ? sess.eventCount : logged)), ui.showAll ? " events" : ` ${plural(logged, "violation")} logged`);
  setAttr(v.tchips, "hidden", !ui.showAll);
  const chipCounts = { changes: n("change"), commands: n("execute"), outside: n("outside") + n("ask") + n("delegate"), failures: tools.filter(e => e.failed).length };
  for (const c of v.tchips.children) updateEventChip(c, chipCounts[c.dataset.i]);
  if (v.showAll.checked !== ui.showAll) v.showAll.checked = ui.showAll;
  const slot = data.sessionFindings;
  const loaded = detail?.loaded === true;
  const violations = slot.owner === sess.sessionId ? slot.rows.length : 0;
  renderTimeline(v.tl, v.rail, sess, turns, cs, {
    loaded,
    total,
    earlier: loaded && from > -Infinity && {
      events: detail.events.length, eventCount: Math.max(sess.eventCount, detail.events.length), violations, total: total ?? violations,
    },
  });
}

/* ---------- actions ---------- */

function toggleSet(set, key) {
  if (set.has(key)) set.delete(key); else set.add(key);
}

export const actions = {
  ssel: id => { ui.evFilter = "all"; selectSession(id); },
  copyid: id => copyText(id, null, `Copied ${id}`),
  jump: id => jumpTo(id),
  etoggle: (id, b) => {
    const wasOpen = b.parentElement.dataset.open === "true";
    if (wasOpen) { ui.openEvents.delete(id); ui.closedEvents.add(id); } else { ui.openEvents.add(id); ui.closedEvents.delete(id); }
    invalidate();
  },
  foldtoggle: id => { toggleSet(ui.openFolds, id); invalidate(); },
  evfilter: id => { ui.evFilter = id; invalidate(); },
  showall: () => { ui.showAll = true; ui.evFilter = "all"; invalidate(); },
  sfilter: id => { ui.sessFilter = id; invalidate(); },
  daytoggle: id => { toggleSet(ui.openDays, id); invalidate(); },
  sessmore: () => loadMore(loadMoreSessions),
  earlier: () => { const s = currentSession(); if (s) loadMore(() => loadEarlier(s.sessionId)); },
  resumetoggle: () => { ui.resumeOpen = !ui.resumeOpen; invalidate(); },
  slisttoggle: () => { listHidden = !listHidden; invalidate(); },
  resumecopy: () => {
    const s = currentSession(); if (!s) return;
    const rc = resumeCommand(s);
    copyText(rc.cmd ?? s.sessionId, document.getElementById("resumeText"), rc.cmd ? "Copied. Run it in your terminal." : "Copied the session id.");
  },
};

export function onInput(el) {
  if (el.id === "sq") { ui.sessQuery = el.value; invalidate(); }
}

export function onChange(el) {
  if (el.id === "showAll") { ui.showAll = el.checked; ui.evFilter = "all"; invalidate(); }
  else if (el.id === "spick" || el.id === "hpick") selectSession(el.value);
}

export function onResize() {
  placeTicks(document.getElementById("rail"), document.getElementById("tl"));
}
