import { h, setAttr, setText, setClass, chevron, syncChildren } from "../dom.js";
import { fmt, plural, ago, clockSeconds, shortPath, truncate } from "../format.js";
import { violationOutcome } from "../derive.js";
import { ui, titleOf, packRuleOf } from "../state.js";
import { openState, lastEventAt, droppedEvents, uncheckedBoundaries, codeRulesPendingBoundaries, ignoredEdits, ignoreRuleChanges, outputText, describeInput } from "../sessions-model.js";

/** Every event with "Show all events", else only those that carry a shown violation. */
const EF = () => (ui.showAll ? "all" : "corrections");
/** A violation's card shows unless the timeline is filtered to another rule (`ui.sessRule`). */
const cardShown = c => ui.sessRule === null || c.finding.ruleId === ui.sessRule;
const hasShownCard = ev => ev.corrections.some(cardShown);
/** Folded away as noise, unless a shown correction hangs off it: that one must show, whatever its level. */
const QUIET = ev => (ev.level === "observe" || ev.level === "coordinate") && !hasShownCard(ev);

const evMatches = ev => EF() === "all" || hasShownCard(ev);

let t = null;

function build(tl) {
  t = {
    empty: h("div.empty", { style: "padding:48px 20px", hidden: true }, h("h3", null, "No violations logged."), h("p"), h("p", { style: "margin-top:12px" }, h("button.btn.sm", { "data-act": "showall" }, "Show all events"))),
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
    h("span.spk", { hidden: true }),
    h("div.pay"));
}

/** Who a call or card is from ("Agent", "Rerail"), in the column "You" sits in; `null` continues the row above's. */
function setSpeaker(node, who) {
  const spk = node.querySelector(":scope > .spk");
  setAttr(spk, "hidden", !who);
  setText(spk, who ?? "");
}

function verbLabel(ev) {
  if (ev.verb === "run" && ev.kind !== "command") return `Run · ${ev.kind}`;
  if (ev.verb === "other") return ev.toolName;
  return ev.label;
}

function updateEvent(node, ev, who = null) {
  setSpeaker(node, who);
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
  const tm = eh.querySelector(".tm");
  setText(tm, `${clockSeconds(ev.callTime)}${took}`);
  setAttr(tm, "title", `Call #${ev.callSeq}${resultRef} · ${ev.toolName}`);
  const pay = node.lastChild;
  const paySig = isOpen ? `${ev.key}|${ev.resSeq}` : "";
  if (pay.dataset.sig !== paySig) { pay.dataset.sig = paySig; pay.replaceChildren(...(isOpen ? payload(ev) : [])); }
}

/** A violation's card: its rule's title, which opens the rule, the rule id, then its badges, over what Rerail said. */
function createCorr(item) {
  return h("div.corr", { id: `c-${item.key}`, "data-key": item.key },
    h("span.spk", { hidden: true }),
    h("div.ch", null, h("button.ctitle"), h("span.mono"),
      h("span.badges", null, h("span.badge.outcome"),
        h("span.badge.declined", { hidden: true, title: "The coding agent didn’t follow this steer and recorded why with rerail feedback." }, "Declined"))),
    h("dl.beats", null, h("dt.nera", null, "Rerail said"), h("dd.said"),
      h("dt.reason", { hidden: true }, "Agent said"), h("dd.reason", { hidden: true }),
      h("dt.where", null, "Where"), h("dd.where", null, h("code"))));
}

/** A declined steer's badge and the agent's reasons under Rerail's message, one per line; hidden for a card no note answers. */
function updateDeclined(node, reasons) {
  const declined = reasons !== null;
  setAttr(node.querySelector(".badge.declined"), "hidden", !declined);
  setAttr(node.querySelector("dt.reason"), "hidden", !declined);
  const said = node.querySelector("dd.reason");
  setAttr(said, "hidden", !declined);
  setText(said, declined ? reasons.join("\n") : "");
}

/** A path as text that a narrow card breaks after a `/` first (`<wbr>`), so a folder name stays whole where it can. */
function setPath(code, text) {
  if (code.dataset.text === text) return;
  code.dataset.text = text;
  code.replaceChildren(...text.split("/").flatMap((part, i, parts) => (i < parts.length - 1 ? [`${part}/`, document.createElement("wbr")] : [part])));
}

/** `path:line` of a finding, or `""` when it has no path. */
function whereText(f) {
  if (!f.path) return "";
  const line = f.span ? `:${f.span.startLine + 1}` : "";
  return `${f.path}${line}`;
}

/**
 * The card's title, which opens the rule. A rule taken out of the repository since keeps its card: named from its
 * pack, else by id, as plain text with nothing to open.
 */
function updateTitle(title, rule, ruleId) {
  if (rule === null) {
    setText(title, `${packRuleOf(ruleId)?.rule.title ?? ruleId} (no longer in this repository)`);
    for (const name of ["data-act", "data-i", "title"]) setAttr(title, name, null);
    setAttr(title, "disabled", true);
    return;
  }
  setText(title, titleOf(rule));
  setAttr(title, "data-act", "detail");
  setAttr(title, "data-i", ruleId);
  setAttr(title, "title", "Open the rule");
  setAttr(title, "disabled", false);
}

function updateCorr(node, item) {
  const c = item.c, f = c.finding;
  setSpeaker(node, item.who);
  updateTitle(node.querySelector(".ctitle"), c.rule, f.ruleId);
  setText(node.querySelector(".mono"), f.ruleId);
  const result = violationOutcome(f);
  node.dataset.outcome = result;
  const badge = node.querySelector(".badge.outcome");
  setText(badge, result === "logged" ? "Logged" : "Steered");
  setAttr(badge, "title", result === "logged" ? "Rerail recorded it; the coding agent wasn’t told." : "Rerail told the coding agent.");
  updateDeclined(node, c.declined);
  setText(node.querySelector(".said"), f.message);
  const where = whereText(f);
  setPath(node.querySelector("dd.where code"), where);
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

/** The prompt or reply row of a turn, hidden when the turn has none. */
function updateMessage(row, text, seq, time) {
  setAttr(row, "hidden", text === null);
  if (text === null) return;
  setText(row.querySelector(".msg"), text);
  setText(row.querySelector(".mt"), `#${seq} · ${clockSeconds(time)}`);
}

/** The fold of a turn's quiet calls (reads, searches, plan updates): offered only while every event shows. */
function updateFold(fold, foldBody, turn, sessionId) {
  const quiet = turn.tools.filter(evMatches).filter(QUIET);
  const fkey = `${sessionId}:f${turn.ti}`, fopen = ui.openFolds.has(fkey), showFold = quiet.length > 0 && EF() === "all";
  setAttr(fold, "hidden", !showFold);
  setAttr(fold, "data-i", fkey);
  setAttr(fold, "aria-expanded", String(fopen));
  if (showFold) setText(fold.lastChild, foldSummary(quiet));
  setAttr(foldBody, "hidden", !(showFold && fopen));
  syncChildren(foldBody, showFold && fopen ? quiet : [], e => e.key, createEvent, updateEvent);
}

/**
 * The turn's body: its shown loud calls, each followed by its shown cards. Each item says who it is from (`who`) only
 * where the speaker changes, as a chat names a speaker once per run: the agent's calls after its reply or your
 * prompt, Rerail's cards after them.
 */
function turnItems(turn) {
  const card = c => ({ key: `c:${c.key}`, c });
  // Orphans are older than every loaded call, so they lead the turn.
  const items = turn.orphans.filter(cardShown).map(card);
  for (const ev of turn.tools) {
    if (QUIET(ev) || !evMatches(ev)) continue;
    items.push({ key: `e:${ev.key}`, ev });
    items.push(...ev.corrections.filter(cardShown).map(card));
  }
  let last = turn.a !== null ? "Agent" : turn.u !== null ? "You" : null;
  for (const item of items) {
    const who = item.ev ? "Agent" : "Rerail";
    item.who = who === last ? null : who;
    last = who;
  }
  return items;
}

function updateTurn(node, turn, sessionId) {
  const [u, a, fold, foldBody, body] = node.children;
  updateMessage(u, turn.u, turn.uSeq, turn.uTime);
  updateMessage(a, turn.a === null ? null : truncate(turn.a, 600), turn.aSeq, turn.aTime);
  updateFold(fold, foldBody, turn, sessionId);
  syncChildren(body, turnItems(turn), i => i.key, i => (i.ev ? createEvent(i.ev) : createCorr(i)), (n, i) => (i.ev ? updateEvent(n, i.ev, i.who) : updateCorr(n, i)));
}

const turnVisible = turn => EF() === "all" || turn.tools.some(evMatches) || turn.orphans.some(cardShown);

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

/** What Rerail did not see of the session, in a sentence's clauses: empty when it saw all of it. */
function coverageGaps(sess) {
  const dropped = droppedEvents(sess);
  const unchecked = uncheckedBoundaries(sess);
  const rulesPending = codeRulesPendingBoundaries(sess);
  const ignored = ignoredEdits(sess);
  const ruleChanges = ignoreRuleChanges(sess);
  return [
    ...(dropped === 0 ? [] : [`${fmt(dropped)} ${plural(dropped, "event")} never reached Rerail`]),
    ...(unchecked === 0 ? [] : [`rules had not yet seen shell edits at ${fmt(unchecked)} ${plural(unchecked, "boundary", "boundaries")}`]),
    ...(rulesPending === 0 ? [] : [`code rules were not yet running at ${fmt(rulesPending)} ${plural(rulesPending, "boundary", "boundaries")}`]),
    ...(ignored === 0 ? [] : [`${fmt(ignored)} ${plural(ignored, "edit")} went unchecked because Git ignores the file`]),
    ...(ruleChanges === 0 ? [] : [`Git's ignore rules changed ${ruleChanges === 1 ? "once" : `${fmt(ruleChanges)} times`}`]),
  ].join("; ");
}

/** The audited-clean note, and what it says of the session's coverage. */
function renderCleanNote(sess, gaps, loaded, clean) {
  setAttr(t.empty, "hidden", !(loaded && clean));
  setText(t.empty.querySelector("p"), gaps === ""
    ? `Rerail audited the whole session: ${fmt(sess.eventCount)} events.`
    : `Rerail saw ${fmt(sess.eventCount)} events; ${gaps}.`);
}

/** The rail, the start and the end, once there is something to place them around. */
function renderBounds(rail, sess, gaps, { loaded, quietEmpty, earlier }) {
  const hidden = quietEmpty || !loaded;
  setAttr(rail, "hidden", hidden);
  // The start is only reached once the whole session is loaded back to it.
  setAttr(t.start, "hidden", hidden || earlier !== false);
  setAttr(t.end, "hidden", hidden);
  const end = endLabel(openState(sess), sess);
  setText(t.end, gaps === "" ? end : `${end} · ${gaps}`);
}

function renderLoading(loaded, quietEmpty, total) {
  setAttr(t.loading, "hidden", loaded && !(quietEmpty && total === null));
}

const visibleTurns = (turns, quietEmpty) => (quietEmpty ? [] : turns.filter(turnVisible));

const earlierToOffer = ({ loaded, clean, earlier }) => (loaded && !clean ? earlier : false);

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
  const gaps = coverageGaps(sess);
  renderCleanNote(sess, gaps, loaded, clean);
  renderLoading(loaded, quietEmpty, total);
  renderBounds(rail, sess, gaps, { loaded, quietEmpty, earlier });
  syncChildren(t.turns, visibleTurns(turns, quietEmpty), turn => turn.key, createTurn, (n, turn) => updateTurn(n, turn, sess.sessionId));
  renderEarlier(earlierToOffer({ loaded, clean, earlier }));
  placeTicks(rail, tl);
}

/**
 * How far `el` sits below the top of `root`, an ancestor it is laid out in, by offsets: unlike two viewport rects,
 * this cannot change between reads when the browser re-anchors the scroll as the timeline shrinks (Show all events
 * off, under a scrolled view), which left every tick displaced by the scroll and the timeline padded with empty space.
 */
function offsetWithin(el, root) {
  let y = 0;
  for (let node = el; node !== root; node = node.offsetParent) {
    if (node === null) return el.getBoundingClientRect().top - root.getBoundingClientRect().top;
    y += node.offsetTop;
  }
  return y;
}

/**
 * One rail tick per correction card, in its card's outcome colour, centred on the card's first title line: the line
 * the "Rerail" speaker label sits on. Every position is read before any tick moves, so placing ticks changes no layout
 * the next one is read from.
 */
export function placeTicks(rail, tl) {
  if (!rail || !tl || rail.hidden) return;
  // The rail and the timeline share the positioned wrapper the ticks' offsets are taken in.
  const root = rail.offsetParent ?? rail.parentElement;
  const railTop = offsetWithin(rail, root);
  const placed = [...tl.querySelectorAll(".corr")].map(card => {
    const title = card.querySelector(".ctitle");
    const line = parseFloat(getComputedStyle(title).lineHeight) || 20;
    return { card, top: offsetWithin(title, root) - railTop + line / 2 };
  });
  for (const old of rail.querySelectorAll(".tick")) old.remove();
  for (const { card, top } of placed) {
    const tick = h("button.tick", { "aria-label": "Jump to violation", "data-act": "jump", "data-i": card.dataset.key, "data-outcome": card.dataset.outcome });
    tick.style.top = `${top}px`;
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
