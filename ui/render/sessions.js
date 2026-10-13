import { h, setAttr, setText, setClass, chevron, searchIcon, syncChildren } from "../dom.js";
import { fmt, plural, ago, duration, stamp, dayLabel, daysAgo, shortSessionId } from "../format.js";
import { data, ui } from "../state.js";
import { ensureTitle } from "../poll.js";
import { sessionTurns, sessionCorrections, placeCorrections, resumeCommand, openState, lastEventAt, isOpen } from "../sessions-model.js";
import { renderTimeline, findingsCover } from "./timeline.js";

let v = null;
/** Session whose day group was last revealed; a new selection opens its day once, then the user owns the fold. */
let revealed = null;

function dayKey(d) {
  if (d < 7) return `d${d}`;
  if (d < 30) return `w${Math.floor(d / 7)}`;
  return "older";
}
export const titleOf = id => data.titles.get(id) ?? "Untitled session";

function build() {
  const el = {};
  el.pick = h("select", { id: "spick", "aria-label": "Choose a session" });
  el.lead = h("p");
  el.search = h("input", { id: "sq", type: "search", placeholder: "Search title or id", "aria-label": "Search sessions" });
  el.chips = h("div.chips", { style: "margin-bottom:8px" },
    h("button.chip", { "data-act": "sfilter", "data-i": "all" }, "All"),
    h("button.chip", { "data-act": "sfilter", "data-i": "corr" }, "With violations"),
    h("button.chip", { "data-act": "sfilter", "data-i": "open" }, "In progress"));
  el.list = h("div.list.slist");
  el.listEmpty = h("div.empty", { hidden: true }, h("p", null, "No sessions match."));
  el.title = h("h2");
  el.state = h("span", { hidden: true });
  el.started = h("span"); el.dur = h("span"); el.events = h("span"); el.sid = h("span.mono");
  el.copyId = h("button.btn.sm", { "data-act": "copyid" }, "Copy id");
  el.resume = h("button.btn.sm.solid", { "data-act": "resumetoggle", "aria-expanded": "false" });
  el.resumeLbl = h("p.lbl");
  el.resumeText = h("textarea.one", { id: "resumeText", readonly: true, "aria-label": "Resume command" });
  el.resumeHow = h("p", { style: "margin:0;font-size:13px", hidden: true });
  el.resumeFoot = h("span");
  el.resumeCopy = h("button.btn.sm.solid", { "data-act": "resumecopy" });
  el.resumeBox = h("div.pask.sresume", { hidden: true }, el.resumeLbl, el.resumeText, el.resumeHow, h("div.foot", null, el.resumeFoot, el.resumeCopy));
  el.tsum = h("span.tsum");
  el.tchips = h("span.tchips", { role: "group", "aria-label": "Event type", hidden: true },
    [["all", "All"], ["changes", "Changes"], ["commands", "Commands"], ["outside", "Outside & asks"], ["failures", "Failures"]].map(([k, l]) =>
      h("button.chip", { "data-act": "evfilter", "data-i": k, "aria-pressed": "false" }, l, k === "all" ? null : h("span.n"))));
  el.showAll = h("input", { type: "checkbox", id: "showAll" });
  el.rail = h("div.rail", { id: "rail" }, h("div.line"));
  el.tl = h("div.tl", { id: "tl" });
  el.scroll = h("div.scroll", { id: "sscroll" }, h("div.tlwrap", null, el.rail, el.tl));
  el.empty = h("div.list", { hidden: true }, h("div.empty", null, h("h3", null, "No coding agent sessions yet"), h("p", null, "Your next Claude Code, Cursor, Codex, OMP or Pi session in this repo shows up here.")));
  el.sess = h("div.sess", null,
    h("aside", null,
      h("div.spick", null, h("label", { for: "spick" }, "Session"), el.pick),
      h("div.ghead", null, h("h2", null, "Sessions"), el.lead),
      h("div.ssearch", null, searchIcon(13), el.search),
      el.chips, el.list, el.listEmpty),
    h("section", null, h("div.list", null,
      h("div.shead", null,
        h("div.l1", null, el.title, el.state),
        h("div.l3", null, el.started, el.dur, el.events, el.sid, h("span.acts", null, el.copyId, el.resume)),
        el.resumeBox),
      h("div.tfilter", null, el.tsum, el.tchips, h("label.tall", null, el.showAll, " Show all events")),
      el.scroll)));
  el.root = h("div", null, el.empty, el.sess);
  v = el;
}

function createListItem(item) {
  if (item.group) {
    return h("button.daygrp", { "data-act": "daytoggle", "data-i": item.dayKey, "aria-expanded": "false" }, chevron(), h("span.name"), h("span.cnt"));
  }
  if (item.more !== undefined) return h("div.smore", null, h("button.btn.sm", { "data-act": "daymore", "data-i": item.dayKey }));
  const s = item.s;
  return h("button.srow", { "data-act": "ssel", "data-i": s.sessionId, "aria-current": "false" },
    h("span.l", null, h("span.t"), h("span.m", null, h("span.state"), h("span.ev"), h("span.sid"))),
    h("span.c.num", null, h("b"), h("span")));
}

function updateListItem(node, item) {
  if (item.group) {
    setAttr(node, "aria-expanded", String(item.open));
    setText(node.querySelector(".name"), item.name);
    const withC = item.withC ? ` · ${fmt(item.withC)} with violations` : "";
    setText(node.querySelector(".cnt"), `${fmt(item.count)} ${plural(item.count, "session")}${withC}`);
    return;
  }
  if (item.more !== undefined) { setText(node.firstChild, `Show 50 more of ${fmt(item.more)}`); return; }
  const s = item.s;
  ensureTitle(s);
  const title = titleOf(s.sessionId);
  setAttr(node, "aria-current", String(s.sessionId === ui.selectedSession));
  const t = node.querySelector(".t");
  setText(t, title); setAttr(t, "title", title);
  const st = openState(s), state = node.querySelector(".state");
  setAttr(state, "class", `state ${st ?? ""}`);
  setText(state, rowState(st, s));
  setAttr(state, "title", st ? "Last event" : "Ended");
  setText(node.querySelector(".ev"), `${fmt(s.eventCount)} events`);
  setText(node.querySelector(".sid"), shortSessionId(s.sessionId, s.host));
  const c = node.querySelector(".c");
  setAttr(c, "hidden", item.n === null);
  setClass(c, "zero", item.n === 0);
  setText(c.firstChild, fmt(item.n ?? 0));
  setText(c.lastChild, plural(item.n ?? 0, "violation"));
}

function rowState(st, s) {
  if (st === "live") return `in progress · ${ago(lastEventAt(s))}`;
  if (st === "stale") return `no end recorded · ${ago(lastEventAt(s))}`;
  return ago(s.endedAt ?? s.startedAt);
}

/** The session the right column shows: the selected one, else the newest. `app.js` selects (and loads) the fallback. */
export function currentSession() {
  return data.sessions.find(s => s.sessionId === ui.selectedSession) ?? data.sessions[0] ?? null;
}

export function renderSessions(root) {
  if (v === null) build();
  if (v.root.parentNode !== root) root.replaceChildren(v.root);
  const all = data.sessions;
  setAttr(v.empty, "hidden", all.length > 0);
  setAttr(v.sess, "hidden", all.length === 0);
  if (all.length === 0) return;
  const sess = currentSession();
  renderList(all, sess);
  renderHeader(sess);
  renderTranscript(sess);
}

/** Trigger counts per session from the fetched findings (the range applies). */
function triggerCounts() {
  const counts = new Map();
  for (const f of data.findings) counts.set(f.sessionId, (counts.get(f.sessionId) ?? 0) + 1);
  return counts;
}

function passesFilter(s, counts) {
  if (ui.sessFilter === "corr") return Boolean(counts.get(s.sessionId));
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

/** The list items one day group contributes: its header, then (when open) up to the shown cap of rows and a "more" row. `n` is `null` when the fetched findings cannot say. */
function groupItems(g, counts, forceOpen) {
  const open = forceOpen || ui.openDays.has(g.key);
  const cap = ui.dayShown[g.key] ?? 50;
  const items = [{ key: `g:${g.key}`, dayKey: g.key, group: true, open, name: g.name, count: g.rows.length, withC: g.rows.filter(s => counts.get(s.sessionId)).length }];
  if (!open) return items;
  for (const s of g.rows.slice(0, cap)) items.push({ key: s.sessionId, s, n: counts.get(s.sessionId) ?? (findingsCover(s) ? 0 : null) });
  if (g.rows.length > cap) items.push({ key: `m:${g.key}`, more: g.rows.length - cap, dayKey: g.key });
  return items;
}

function renderList(all, sess) {
  const counts = triggerCounts();
  const q = ui.sessQuery.trim().toLowerCase();
  const visible = all.filter(s => passesFilter(s, counts) && matchesQuery(s, q));
  setText(v.lead, `${fmt(visible.length)} of ${fmt(all.length)} · newest first`);
  if (v.search.value !== ui.sessQuery && document.activeElement !== v.search) v.search.value = ui.sessQuery;
  for (const c of v.chips.children) setAttr(c, "aria-pressed", String(c.dataset.i === ui.sessFilter));

  if (sess.sessionId !== revealed) { revealed = sess.sessionId; ui.openDays.add(dayKey(daysAgo(sess.startedAt))); }
  const items = groupByDay(visible).flatMap(g => groupItems(g, counts, q !== ""));
  syncChildren(v.list, items, i => i.key, createListItem, updateListItem);
  setAttr(v.listEmpty, "hidden", items.length > 0);
  syncChildren(v.pick, visible.slice(0, 200), s => s.sessionId, s => h("option", { value: s.sessionId }),
    (o, s) => { setText(o, `${titleOf(s.sessionId)} · ${shortSessionId(s.sessionId, s.host)}`); });
  if (v.pick.value !== sess.sessionId) v.pick.value = sess.sessionId;
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
  setText(v.title, titleOf(sess.sessionId));
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
  const events = detail?.events ?? [];
  const cs = sessionCorrections(sess.sessionId);
  const turns = placeCorrections(sessionTurns(events), cs);
  const tools = turns.flatMap(t => t.tools);
  const n = lvl => tools.filter(e => e.level === lvl).length;
  setText(v.tsum, "");
  v.tsum.replaceChildren(h("b", null, fmt(ui.showAll ? sess.eventCount : cs.length)), ui.showAll ? " events" : ` ${plural(cs.length, "violation")} logged`);
  setAttr(v.tchips, "hidden", !ui.showAll);
  const chipCounts = { changes: n("change"), commands: n("execute"), outside: n("outside") + n("ask") + n("delegate"), failures: tools.filter(e => e.failed).length };
  for (const c of v.tchips.children) updateEventChip(c, chipCounts[c.dataset.i]);
  if (v.showAll.checked !== ui.showAll) v.showAll.checked = ui.showAll;
  renderTimeline(v.tl, v.rail, sess, turns, cs, detail?.loaded === true);
}
