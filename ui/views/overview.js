/**
 * Overview (`#overview`, the default; `#rules` before v8): the banner the repository's first-open state calls for
 * (`first-open.js`), the range's four tiles, then the rules with the most violations logged and the newest sessions,
 * or the empty block that says what to do next.
 */
import { h, setText, setAttr, setClass, setContent, syncChildren, icon } from "../dom.js";
import { ago, clock, fmt, plural, shortSessionId, whenLabel, HOST_NAME } from "../format.js";
import { isByRerail, isRunning } from "../derive.js";
import { data, ui, activityAllOf, fires, rangeLabel, repoName, rulesArray, ruleSections, stateHere, titleOf } from "../state.js";
import { ensureTitle } from "../poll.js";
import { invalidate } from "../shell.js";
import { rangeControl, syncRange } from "../render/range.js";
import { bannerFor, firstOpenState, liveState, overviewBody, rulesOnSource, steeredTally, topRules } from "../first-open.js";

export const needs = ["rules", "packs", "activity", "recentSessions"];

/** How many of the range's newest sessions Recent sessions lists. */
const RECENT_SESSIONS = 5;
/** The Since install range (`RANGES` in `state.js`): what a range without violations offers to widen to. */
const ALL_TIME = 9999;

/* ---------- what the page knows ---------- */

/** Rule packs added here with a rule, and rules created by you: the Rules on tile's line and its link. */
function ruleSources() {
  const sections = ruleSections();
  const packs = sections.filter(s => s.kind === "pack");
  const mine = rulesArray().filter(r => !isByRerail(r)).length;
  return { packs, mine };
}

function facts() {
  const sessionsEver = data.activityAllSessions?.started ?? 0;
  const ruleCount = data.rules.size;
  return {
    state: firstOpenState({
      scoped: ui.scope !== null, missing: data.missing, setUp: data.repository?.setUp === true,
      stopped: stateHere() === "stopped", ruleCount, sessionsEver,
    }),
    ruleCount, sessionsEver,
  };
}

/* ---------- banners ---------- */

const welcomeKey = () => `rerail-welcome-dismissed:${data.repository?.repositoryKey ?? ""}`;

function welcomeDismissed() {
  if (ui.welcomeDismissed) return true;
  try { return localStorage.getItem(welcomeKey()) === "1"; } catch { return false; /* storage blocked */ }
}

function dismissWelcome() {
  ui.welcomeDismissed = true;
  try { localStorage.setItem(welcomeKey(), "1"); } catch { /* private mode: dismissed for this visit */ }
}

function stoppedBanner(repo) {
  return h("div.welcome.warn", { role: "status" }, icon("steered", 22), h("div", null,
    h("b", null, `Rerail is stopped in ${repo}.`),
    h("p", null, "Nothing is audited or logged here; your rules and history are kept. Starting it here starts every checkout of the repo; ",
      h("span.mono", null, "/rerail-start"), " in a session starts that checkout."),
    h("p.acts", null, h("button.btn.sm.solid", { "data-act": "repostart", "data-i": data.repository.repositoryKey }, "Start Rerail"))));
}

function welcomeBanner(repo, packCount) {
  const on = rulesArray().filter(isRunning).length;
  const from = packCount > 0 ? ` from ${fmt(packCount)} rule ${plural(packCount, "pack")}` : "";
  return h("div.welcome", { role: "status" }, icon("done", 22), h("div", null,
    h("b", null, `Rerail is set up. ${fmt(on)} ${plural(on, "rule")}${from} ${on === 1 ? "is" : "are"} on.`),
    h("p", null, `Open a new coding agent session in ${repo} and start coding. Sessions already open in Claude Code, Codex or Cursor pick up the rules from their next step; in OMP or Pi, start a new session or type `,
      h("span.mono", null, "/rerail-start"), " in the open one.")),
  h("button.x", { "data-act": "dismisswelcome", "aria-label": "Dismiss" }, "✕"));
}

function renderBanner(root, state, packCount) {
  const kind = data.loaded ? bannerFor(state, welcomeDismissed()) : null;
  const repo = repoName();
  const on = rulesArray().filter(isRunning).length;
  setContent(root, JSON.stringify([kind, repo, on, packCount, data.repository?.repositoryKey]), () =>
    kind === "stopped" ? stoppedBanner(repo) : kind === "welcome" ? welcomeBanner(repo, packCount) : null);
}

/* ---------- tiles ---------- */

/** Each tile lands on the list that produced its number; Rules on opens the first rule pack (`rulesOnHash`). */
const TILES = [
  { key: "steered", label: "Violations steered", title: "Open History by rule, steered only" },
  { key: "sessions", label: "Sessions with violations logged", title: "Open History by session, with violations" },
  { key: "fired", label: "Rules with violations logged", title: "Open History by rule" },
  { key: "on", label: "Rules on", title: "Open your rule packs" },
];

const tile = spec => h("button.tile", { "data-act": "nav", "data-tile": spec.key, title: spec.title },
  h("div.k", null, spec.label), h("div.v.num", null, h("span.big"), h("small")), h("div.s"));

/** A figure, " of <of>" beside it when there is something to count against, and the line under it. */
function setTile(el, hash, big, of, line) {
  setAttr(el, "data-i", hash);
  setText(el.querySelector(".big"), fmt(big));
  setText(el.querySelector("small"), of > 0 ? ` of ${fmt(of)}` : "");
  setText(el.querySelector(".s"), line);
}

/** Rules on opens the sidebar's first rule source: Created by you, else the first pack, else the catalog. */
function rulesOnHash({ packs, mine }) {
  if (mine > 0) return "created";
  const first = [...packs].sort((a, b) => a.pack.name.localeCompare(b.pack.name, undefined, { sensitivity: "base" }))[0];
  return first === undefined ? "packs" : `pack/${first.pack.id}`;
}

function renderTiles(tiles, sources, ruleCount) {
  const label = rangeLabel();
  const rules = rulesArray();
  const on = rules.filter(isRunning).length;
  const { steered, total } = steeredTally(data.activity);
  setTile(tiles[0], "history?outcome=steered", steered, total, total > 0 ? label : ruleCount === 0 ? "nothing to steer yet" : `none ${label}`);

  const started = data.activitySessions?.started ?? 0;
  const sessionsEver = data.activityAllSessions?.started ?? 0;
  setTile(tiles[1], "sessions?filter=violations", started > 0 ? data.activitySessions.withViolations : 0, started,
    started > 0 ? label : sessionsEver > 0 ? `no sessions ${label}` : "your next coding agent session shows here");

  const fired = rules.filter(r => fires(r.rule.id) > 0).length;
  setTile(tiles[2], "history", fired, on, fired > 0 ? label : `none ${label}`);

  setTile(tiles[3], rulesOnHash(sources), on, rules.length, rulesOnSource(sources.packs.length, sources.mine));
}

/* ---------- Most violations logged ---------- */

const colhead = () => h("div.colhead", { "aria-hidden": "true" }, h("span", null, "Rule"), h("span", null, "Violations"), h("span", null, "On/off"));

function ruleRow(item) {
  return h("div.row", { "data-act": "detail", "data-i": item.id },
    h("div.rmain", null,
      h("div.rtitle", null, h("button.t", { "data-act": "detail", "data-i": item.id })),
      h("div.rside", null,
        h("span.count.num", null, h("b")),
        h("button.sw", { role: "switch", "data-act": "toggle", "data-i": item.id })),
      h("div.rmeta", null, h("span.mono", null, item.id), h("span.last"))));
}

function updateRuleRow(row, item) {
  const title = row.querySelector("button.t");
  setText(title, item.title);
  setAttr(title, "title", item.title);
  setClass(row, "off", !isRunning(item.stored));
  setText(row.querySelector(".count b"), fmt(item.fires));
  setAttr(row.querySelector(".count"), "title", `${fmt(item.fires)} ${plural(item.fires, "violation")} logged ${rangeLabel()}`);
  const sw = row.querySelector(".sw");
  setAttr(sw, "aria-checked", String(item.stored.enabled));
  setAttr(sw, "aria-label", `Toggle ${item.title}`);
  // Every listed rule logged a violation in range, so it has a last sighting; since install it is the newest one.
  const last = activityAllOf(item.id).lastSeenAt;
  const lastEl = row.querySelector(".last");
  setText(lastEl, last === null ? "" : `last logged ${whenLabel(last)}`);
  lastEl.hidden = last === null;
}

/* ---------- Recent sessions ---------- */

/** "in progress · 4m ago", "no end recorded · 2h ago", else when it started: "today 14:20". */
function whenText(s) {
  const live = liveState(s, Date.now());
  const last = s.lastEventAt ?? s.startedAt;
  if (live === "live") return { cls: "live", text: `in progress · ${ago(last)}` };
  if (live === "stale") return { cls: "stale", text: `no end recorded · ${ago(last)}` };
  return { cls: "", text: `${whenLabel(s.startedAt)} ${clock(s.startedAt)}` };
}

const sessionRow = s => h("button.sesrow", { "data-act": "session", "data-i": s.sessionId, title: "Open this session" },
  h("span.l", null, h("span.t"), h("span.m", null, h("span.host"), h("span.when"), h("span.sid"))),
  h("span.n.num", null, h("b")));

function updateSessionRow(row, s) {
  setText(row.querySelector(".t"), data.titles.get(s.sessionId) ?? "Untitled session");
  setText(row.querySelector(".host"), HOST_NAME[s.host] ?? s.host);
  const when = whenText(s);
  const whenEl = row.querySelector(".when");
  setText(whenEl, when.text);
  setClass(whenEl, "live", when.cls === "live");
  setClass(whenEl, "stale", when.cls === "stale");
  setText(row.querySelector(".sid"), shortSessionId(s.sessionId, s.host));
  const cell = row.querySelector(".n");
  setClass(cell, "zero", s.violationCount === 0);
  setText(cell.querySelector("b"), fmt(s.violationCount));
  setAttr(cell, "title", `${fmt(s.violationCount)} ${plural(s.violationCount, "violation")} logged`);
}

/* ---------- empty blocks ---------- */

function emptyBlock(body, repo) {
  const label = rangeLabel();
  if (body === "no-rules") {
    return h("div.list", null, h("div.empty", null,
      h("h3", null, "No rules added yet"),
      h("p", null, `Nothing is checked in ${repo} yet. Add rule packs, or write a rule of your own with your coding agent. Once rules are on, start a coding agent session in this repo and the violations they log show here.`),
      h("p.acts", null,
        h("button.btn.sm.solid", { "data-act": "nav", "data-i": "packs" }, icon("plus"), "Add rule packs"),
        h("button.btn.sm", { "data-act": "nav", "data-i": "create" }, "Create a rule"))));
  }
  if (body === "no-sessions") {
    const on = rulesArray().filter(isRunning).length;
    return h("div.list", null, h("div.empty", null,
      h("h3", null, on > 0 ? "Rules are on. No violations logged yet." : "Every rule is off."),
      h("p", null, on > 0
        ? `${fmt(on)} ${plural(on, "rule is", "rules are")} watching ${repo}. Start a coding agent session in this repo and the violations they log show here, grouped by rule.`
        : `Switch rules on in their rule packs; the violations they log in coding agent sessions in ${repo} show here.`)));
  }
  return h("div.list", null, h("div.empty", null,
    h("h3", null, `No violations logged ${label}`),
    h("p", null, ui.range === ALL_TIME ? "Wait for the next coding agent session." : "Widen the range, or wait for the next coding agent session."),
    ui.range === ALL_TIME ? null : h("p.acts", null, h("button.btn.sm", { "data-act": "range", "data-i": ALL_TIME }, "Show since install"))));
}

/* ---------- the view ---------- */

let page = null;

function build() {
  const banner = h("div.banners");
  const range = rangeControl();
  const tiles = TILES.map(tile);
  const empty = h("div.ovempty");
  const topList = h("div.list");
  const topNote = h("p");
  const top = h("section.ovtop", null,
    h("div.ghead", null, h("h2", null, "Most violations logged"), topNote,
      h("div.act", null, h("button.btn.sm.ghost.link", { "data-act": "nav", "data-i": "history" }, "View all"))),
    colhead(), topList);
  const recentNote = h("p");
  const recentAll = h("button.btn.sm.ghost.link", { "data-act": "nav", "data-i": "sessions" });
  const recentList = h("div.list");
  const recent = h("section.ovrecent", null,
    h("div.ghead", null, h("h2", null, "Recent sessions"), recentNote, h("div.act", null, recentAll)),
    h("div.colhead.srhead", { "aria-hidden": "true" }, h("span", null, "Session"), h("span", null, "Violations")),
    recentList);
  const lists = h("div.ovlists", null, top, recent);
  const root = h("div.ov", null, banner,
    h("div.rollup", null, h("div.rollhead", null, h("h2", null, "Overview"), range), h("div.tiles", null, tiles)),
    empty, lists);
  page = { root, banner, range, tiles, empty, lists, topList, topNote, recent, recentNote, recentAll, recentList };
}

function renderLists(top, repo) {
  const label = rangeLabel();
  setText(page.topNote, `Rules ranked by violations logged, ${label}.`);
  syncChildren(page.topList, top, item => item.id, ruleRow, updateRuleRow);

  const recent = data.recentSessions;
  const rows = (recent?.rows ?? []).slice(0, RECENT_SESSIONS);
  rows.forEach(ensureTitle);
  page.recent.hidden = rows.length === 0;
  setText(page.recentNote, `Sessions in ${repo}, newest first, ${label}.`);
  setText(page.recentAll, `View all ${fmt(recent?.total ?? 0)}`);
  syncChildren(page.recentList, rows, s => s.sessionId, sessionRow, updateSessionRow);
}

export function render(root) {
  if (page === null) build();
  if (page.root.parentNode !== root) root.replaceChildren(page.root);
  const { state, ruleCount, sessionsEver } = facts();
  const sources = ruleSources();
  const repo = repoName();
  renderBanner(page.banner, state, sources.packs.length);
  syncRange(page.range);
  renderTiles(page.tiles, sources, ruleCount);

  const top = topRules(rulesArray().map(stored => ({ id: stored.rule.id, title: titleOf(stored), fires: fires(stored.rule.id), stored })));
  const body = data.loaded ? overviewBody({ ruleCount, sessionsEver, topCount: top.length }) : "loading";
  page.lists.hidden = body !== "lists";
  page.empty.hidden = body === "lists" || body === "loading";
  if (body === "lists") renderLists(top, repo);
  else if (body !== "loading") {
    const on = rulesArray().filter(isRunning).length;
    setContent(page.empty, JSON.stringify([body, repo, on, ui.range]), () => emptyBlock(body, repo));
  }
}

export const actions = {
  dismisswelcome: () => { dismissWelcome(); invalidate(); },
};
