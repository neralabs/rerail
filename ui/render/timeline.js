import { h, setAttr, setText, setClass, chevron, syncChildren } from "../dom.js";
import { fmt, plural, ago, clockSeconds, shortPath, truncate } from "../format.js";
import { violationOutcome } from "../derive.js";
import { ui, titleOf, packRuleOf } from "../state.js";
import { openState, lastEventAt, droppedEvents, uncheckedBoundaries, codeRulesPendingBoundaries, outputText, describeInput } from "../sessions-model.js";

const EF = () => (ui.showAll ? ui.evFilter : "corrections");
/** Folded away as noise, unless a correction hangs off it: that one must show, whatever its level. */
const QUIET = ev => (ev.level === "observe" || ev.level === "coordinate") && ev.correction === null;

const evMatches = ev => {
  const f = EF();
  return f === "all" || (f === "changes" && ev.level === "change") || (f === "commands" && ev.level === "execute")
    || (f === "outside" && (ev.level === "outside" || ev.level === "ask" || ev.level === "delegate"))
    || (f === "failures" && ev.failed) || (f === "corrections" && ev.correction !== null);
};

let t = null;

function build(tl) {
  t = {
    empty: h("div.empty", { style: "padding:48px 20px", hidden: true }, h("h3", null, "No violations logged"), h("p"), h("p", { style: "margin-top:12px" }, h("button.btn.sm", { "data-act": "showall" }, "Show all events"))),
    loading: h("div.empty", { hidden: true }, h("p", null, "Loading…")),
    earlier: h("div.smore", { hidden: true }, h("button.btn.sm", { "data-act": "earlier" })),
    start: h("div.bound", null, "Session started"),
    turns: h("div.turns"),
    end: h("div.bound"),
  };
  tl.replaceChildren(t.empty, t.loading, t.earlier, t.start, t.turns, t.end);
}

/* ---------- event rows ---------- */

function outcome(ev) {
  if (ev.resSeq === null) return [h("span.dim", null, "…")];
  if (ev.failed) return [h("span.bad", null, "failed")];
  if (ev.verb === "read" && ev.lines) return [`${fmt(ev.lines)} ${plural(ev.lines, "line")}`];
  if (ev.verb === "run") return [h("span.ok", null, "ok")];
  return [h("span.ok", null, "ok")];
}

const str = v => (typeof v === "string" ? v : "");

/** Verb-specific head rows of the payload table. */
function inputRows(ev, i, row) {
  if (ev.verb === "run") { row("Description", str(i.description)); row("Kind", ev.kind); }
  else if (ev.verb === "search" || ev.verb === "find") { row(ev.verb === "find" ? "Glob" : "Pattern", ev.target, true); row("In", str(i.path), true); }
  else if (ev.verb === "web") row("URL", ev.target, true);
  else if (ev.verb === "ask") { row("Question", ev.target); row("Options", Array.isArray(i.options) ? i.options.join(" · ") : ""); }
  else if (ev.verb === "agent") { row("Task", ev.target); row("Type", typeof i.subagent_type === "string" ? i.subagent_type : str(i.agent)); }
  else row("Path", ev.target, true);
}

function resultCell(ev) {
  if (ev.resSeq === null) return "no result recorded";
  return ev.failed ? h("span.bad", null, "failed") : h("span.ok", null, "ok");
}

const diffLines = (cls, sign, text) => text.split("\n").map(l => h(cls, null, `${sign} ${l}`));

/** The command line, or the diff of what was written; `null` when the verb carries neither. */
function commandOrDiff(ev, i) {
  if (ev.verb === "run") return h("pre", null, `$ ${ev.target}`);
  if (typeof i.old_string === "string" && typeof i.new_string === "string") {
    return h("div.diff", null, diffLines("div.d", "-", i.old_string), diffLines("div.a", "+", i.new_string));
  }
  if (typeof i.content === "string" && (ev.verb === "create" || ev.verb === "edit")) {
    return h("div.diff", null, diffLines("div.a", "+", truncate(i.content, 4000)));
  }
  return null;
}

function rawPayload(ev) {
  const pair = ev.resSeq === null ? "" : ` · Result #${ev.resSeq}`;
  return h("details", null, h("summary", null, `Raw · Call #${ev.callSeq}${pair}`),
    h("div.in", null, ev.raw.map(e => h("pre", null, describeInput(e)))));
}

function payload(ev) {
  const i = ev.input && typeof ev.input === "object" ? ev.input : {};
  const rows = [];
  const row = (k, val, mono) => {
    if (val === undefined || val === null || val === "") return;
    rows.push(h("dt", null, k), h("dd", { class: mono ? "mono" : null }, val));
  };
  inputRows(ev, i, row);
  row("Tool", ev.toolName, true);
  row("Result", resultCell(ev));
  if (ev.dur !== null) row("Took", `${ev.dur}s`);
  const parts = [h("dl", null, rows)];
  const body = commandOrDiff(ev, i);
  if (body !== null) parts.push(body);
  const out = outputText(ev.output);
  if (out) parts.push(h("pre", null, truncate(out, 2000)));
  parts.push(rawPayload(ev));
  return parts;
}

function createEvent(ev) {
  return h("div.evt", null,
    h("button.eh", { "data-act": "etoggle", "data-i": ev.key, "aria-expanded": "false" }, chevron(), h("span.vb"), h("span.tg"),
      h("span.out", null, h("span.oc"), h("span.tn"), h("span.sq.num"), h("span.tm.num"))),
    h("div.pay"));
}

function verbLabel(ev) {
  if (ev.verb === "run" && ev.kind !== "command") return `Run · ${ev.kind}`;
  if (ev.verb === "other") return ev.toolName;
  return ev.label;
}

function updateEvent(node, ev) {
  const isOpen = ui.openEvents.has(ev.key) || (ev.failed && !ui.closedEvents.has(ev.key));
  setAttr(node, "class", `evt lvl-${ev.level}${ev.failed ? " failed" : ""}`);
  setAttr(node, "data-open", String(isOpen));
  const eh = node.firstChild;
  setAttr(eh, "aria-expanded", String(isOpen));
  setText(eh.querySelector(".vb"), verbLabel(ev));
  const tg = eh.querySelector(".tg");
  const desc = ev.title !== ev.target;
  setClass(tg, "desc", desc);
  setText(tg, desc ? ev.title : shortPath(ev.title));
  setAttr(tg, "title", ev.target);
  const sig = `${ev.resSeq}|${ev.failed}|${ev.lines}`;
  const oc = eh.querySelector(".oc");
  if (oc.dataset.sig !== sig) { oc.dataset.sig = sig; oc.replaceChildren(...outcome(ev)); }
  const tn = eh.querySelector(".tn"); setText(tn, ev.toolName); setAttr(tn, "title", ev.toolName);
  const sq = eh.querySelector(".sq"); setText(sq, ev.resSeq === null ? `#${ev.callSeq}` : `#${ev.callSeq}–${ev.resSeq}`);
  const resultRef = ev.resSeq === null ? "" : `, Result #${ev.resSeq}`;
  setAttr(sq, "title", `Call #${ev.callSeq}${resultRef}`);
  const took = ev.dur ? ` · ${ev.dur}s` : "";
  setText(eh.querySelector(".tm"), `${clockSeconds(ev.callTime)}${took}`);
  const pay = node.lastChild;
  const paySig = isOpen ? `${ev.key}|${ev.resSeq}` : "";
  if (pay.dataset.sig !== paySig) { pay.dataset.sig = paySig; pay.replaceChildren(...(isOpen ? payload(ev) : [])); }
}

function createCorr(item) {
  return h("div.corr", { id: `c-${item.key}`, "data-key": item.key },
    h("div.ch", null, h("b"), h("span.mono"), h("span.badge"), h("div.act", null, h("button.btn.sm.ghost", { "data-act": "detail" }, "Open rule"))),
    h("dl.beats", null, h("dt.nera", null, "Rerail said"), h("dd.said"), h("dt.where", null, "Where"), h("dd.where", null, h("code"))));
}

/** `path:line` of a finding, or `""` when it has no path. */
function whereText(f) {
  if (!f.path) return "";
  const line = f.span ? `:${f.span.startLine + 1}` : "";
  return `${f.path}${line}`;
}

function updateCorr(node, item) {
  const c = item.c, f = c.finding;
  // A rule taken out of the repository since keeps its card: named from its pack, else by id, with nothing to open.
  setText(node.querySelector("b"), c.rule === null ? `${packRuleOf(f.ruleId)?.rule.title ?? f.ruleId} (no longer in this repository)` : titleOf(c.rule));
  setText(node.querySelector(".mono"), f.ruleId);
  setText(node.querySelector(".badge"), violationOutcome(f) === "logged" ? "Logged" : "Steered");
  setAttr(node.querySelector("[data-act=detail]"), "data-i", f.ruleId);
  setAttr(node.querySelector("[data-act=detail]"), "hidden", c.rule === null);
  setText(node.querySelector(".said"), f.message);
  const where = whereText(f);
  setText(node.querySelector("dd.where code"), where);
  setAttr(node.querySelector("dt.where"), "hidden", where === "");
  setAttr(node.querySelector("dd.where"), "hidden", where === "");
}

/* ---------- turns ---------- */

function createTurn(turn) {
  return h("div.turn", null,
    h("div.msgrow.u", null, h("span.who", null, "You"), h("div.msg"), h("span.mt", { title: "user-prompt" })),
    h("div.msgrow.a", null, h("span.who", null, "Agent"), h("div.msg"), h("span.mt", { title: "assistant-message" })),
    h("button.fold", { "data-act": "foldtoggle", "aria-expanded": "false", hidden: true }, chevron(), h("span")),
    h("div.foldbody", { hidden: true }),
    h("div.tbody"));
}

function foldSummary(quiet) {
  const cnt = verb => quiet.filter(e => e.verb === verb).length, parts = [];
  if (cnt("read")) parts.push(`read ${cnt("read")} ${plural(cnt("read"), "file")}`);
  if (cnt("search")) parts.push(`searched ${cnt("search")}×`);
  if (cnt("find")) parts.push(`listed ${cnt("find")}×`);
  if (cnt("web")) parts.push(`fetched ${cnt("web")} ${plural(cnt("web"), "page")}`);
  if (cnt("todo")) parts.push("updated the plan");
  return parts.join(" · ").replace(/^./, c => c.toUpperCase());
}

function updateTurn(node, turn, sessionId) {
  const [u, a, fold, foldBody, body] = node.children;
  setAttr(u, "hidden", turn.u === null);
  if (turn.u !== null) { setText(u.querySelector(".msg"), turn.u); setText(u.querySelector(".mt"), `#${turn.uSeq} · ${clockSeconds(turn.uTime)}`); }
  setAttr(a, "hidden", turn.a === null);
  if (turn.a !== null) { setText(a.querySelector(".msg"), truncate(turn.a, 600)); setText(a.querySelector(".mt"), `#${turn.aSeq} · ${clockSeconds(turn.aTime)}`); }
  const shown = turn.tools.filter(evMatches);
  const quiet = shown.filter(QUIET), loud = turn.tools.filter(e => !QUIET(e));
  const fkey = `${sessionId}:f${turn.ti}`, fopen = ui.openFolds.has(fkey), showFold = quiet.length > 0 && EF() === "all";
  setAttr(fold, "hidden", !showFold);
  setAttr(fold, "data-i", fkey);
  setAttr(fold, "aria-expanded", String(fopen));
  if (showFold) setText(fold.lastChild, foldSummary(quiet));
  setAttr(foldBody, "hidden", !(showFold && fopen));
  syncChildren(foldBody, showFold && fopen ? quiet : [], e => e.key, createEvent, updateEvent);
  const items = [];
  for (const ev of loud) {
    if (!evMatches(ev)) continue;
    items.push({ key: `e:${ev.key}`, ev });
    if (ev.correction) items.push({ key: `c:${ev.correction.key}`, c: ev.correction });
  }
  for (const c of turn.orphans) items.push({ key: `c:${c.key}`, c });
  syncChildren(body, items, i => i.key, i => (i.ev ? createEvent(i.ev) : createCorr(i)), (n, i) => (i.ev ? updateEvent(n, i.ev) : updateCorr(n, i)));
}

const turnVisible = turn => {
  const f = EF();
  if (f === "all") return true;
  return turn.tools.some(evMatches) || (f === "corrections" && turn.orphans.length > 0);
};

function endLabel(st, sess) {
  if (st === null) return "Session ended";
  if (st === "live") return `In progress · last event ${ago(lastEventAt(sess))}`;
  return `No end recorded · last event ${ago(lastEventAt(sess))}`;
}

/** Load earlier, while the transcript or the session's violations reach less far back than the session's start. */
function renderEarlier(earlier) {
  setAttr(t.earlier, "hidden", earlier === false);
  if (earlier === false) return;
  const parts = [
    ...(earlier.events < earlier.eventCount ? [`${fmt(earlier.events)} of ${fmt(earlier.eventCount)} events`] : []),
    ...(earlier.violations < earlier.total ? [`${fmt(earlier.violations)} of ${fmt(earlier.total)} violations`] : []),
  ];
  setText(t.earlier.firstChild, ["Load earlier", ...parts].join(" · "));
}

/**
 * `page`: whether the newest transcript page is `loaded`, the session's violation `total` (`null` until known), and
 * `earlier`: `false` once the transcript and violations reach the session's start, else how many of each are loaded.
 */
export function renderTimeline(tl, rail, sess, turns, cs, page) {
  if (t === null || tl.firstChild !== t.empty) build(tl);
  const { loaded, total, earlier } = page;
  const quietEmpty = !ui.showAll && cs.length === 0;
  // Audited clean: no violation logged in the whole session, not just none among the loaded events.
  const clean = quietEmpty && total === 0;
  setAttr(t.empty, "hidden", !(loaded && clean));
  const dropped = droppedEvents(sess);
  const unchecked = uncheckedBoundaries(sess);
  const rulesPending = codeRulesPendingBoundaries(sess);
  const gaps = [
    ...(dropped === 0 ? [] : [`${fmt(dropped)} ${plural(dropped, "event")} never reached Rerail`]),
    ...(unchecked === 0 ? [] : [`shell edits were not yet checked at ${fmt(unchecked)} ${plural(unchecked, "boundary", "boundaries")}`]),
    ...(rulesPending === 0 ? [] : [`code rules were not yet running at ${fmt(rulesPending)} ${plural(rulesPending, "boundary", "boundaries")}`]),
  ].join("; ");
  setText(t.empty.querySelector("p"), gaps === ""
    ? `Rerail audited all ${fmt(sess.eventCount)} events in this session and logged no violations.`
    : `Rerail logged no violations in the ${fmt(sess.eventCount)} events it audited; ${gaps}.`);
  setAttr(t.loading, "hidden", loaded && !(quietEmpty && total === null));
  setAttr(rail, "hidden", quietEmpty || !loaded);
  // The start is only reached once the whole session is loaded back to it.
  setAttr(t.start, "hidden", quietEmpty || !loaded || earlier !== false);
  setAttr(t.end, "hidden", quietEmpty || !loaded);
  const st = openState(sess);
  setText(t.end, gaps === "" ? endLabel(st, sess) : `${endLabel(st, sess)} · ${gaps}`);
  syncChildren(t.turns, quietEmpty ? [] : turns.filter(turnVisible), turn => turn.key, createTurn, (n, turn) => updateTurn(n, turn, sess.sessionId));
  renderEarlier(loaded && !clean ? earlier : false);
  placeTicks(rail, tl);
}

/** One rail tick per correction card, at the card's offset within the timeline. */
export function placeTicks(rail, tl) {
  if (!rail || !tl || rail.hidden) return;
  const top = tl.getBoundingClientRect().top;
  for (const old of rail.querySelectorAll(".tick")) old.remove();
  for (const card of tl.querySelectorAll(".corr")) {
    const tick = h("button.tick", { "aria-label": "Jump to violation", "data-act": "jump", "data-i": card.dataset.key });
    tick.style.top = `${card.getBoundingClientRect().top - top + 14}px`;
    rail.append(tick);
  }
}

export function jumpTo(key) {
  const el = document.getElementById(`c-${key}`);
  if (!el) return;
  el.scrollIntoView({ behavior: "smooth", block: "center" });
  el.classList.remove("hit");
  el.getBoundingClientRect(); // force a reflow so the "hit" animation restarts
  el.classList.add("hit");
}
