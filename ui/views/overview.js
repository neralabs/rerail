/**
 * Overview (`#overview`, the default; `#rules` before v8): the banner the repository's first-open state calls for
 * (`first-open.js`), the banners that ask for approval, a newer Rerail once the daemon reports one, the range's four tiles, then the rules with the most violations logged and the newest sessions,
 * or the empty block that says what to do next.
 */
import { h, setText, setAttr, setClass, setContent, syncChildren, icon, pendingIcon } from "../dom.js";
import { ago, andList, clock, fmt, plural, shortSessionId, whenLabel, HOST_NAME } from "../format.js";
import { awaitingApproval, awaitingYourApproval, isFromPack, isLog, isRunning, worktreeLeftOut } from "../derive.js";
import { data, ui, ALL_TIME, activityAllOf, allFires, fires, isSetUp, packById, packOfRule, rangeLabel, repoName, rulesArray, ruleSections, stateHere, titleOf } from "../state.js";
import { newerRelease } from "../settings-model.js";
import { ensureTitle } from "../poll.js";
import { invalidate } from "../shell.js";
import { AGENT_UPDATE_COMMAND, updateCopyButton, updateDetailsLink } from "../render/command.js";
import { rangeControl, syncRange } from "../render/range.js";
import { loggedTip } from "../render/rules.js";
import { enforcementTag, ruleTableHead, updateEnforcementTag } from "../render/enforcement.js";
import { bannerFor, firstOpenState, liveState, overviewBody, rulesOnSource, steeredTally, topRules } from "../first-open.js";

export const needs = ["rules", "packs", "activity", "recentSessions"];

/** How many of the range's newest sessions Recent sessions lists. */
const RECENT_SESSIONS = 5;

/* ---------- what the page knows ---------- */

/** Rule packs added here with a rule, and rules created by you: the Rules on tile's line and its link. */
function ruleSources() {
  const sections = ruleSections();
  const packs = sections.filter(s => s.kind === "pack");
  const mine = rulesArray().filter(r => !isFromPack(r)).length;
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

/*
 * The banners for moments where nothing is at stake (day one, a new version, a stop someone may have chosen) carry a
 * little personality in their icon and a word or two. The ones that ask a person to approve, judge or fix something
 * keep plain icons and words.
 */
function stoppedBanner(repo) {
  return h("div.welcome.warn", { role: "status" }, icon("melting", 22), h("div", null,
    h("b", null, `Rerail is stopped in ${repo}.`),
    h("p", null, "No rules run here and nothing is logged, but your rules and history are safe. Start it again whenever you’re ready."),
    h("p.acts", null, h("button.btn.sm.solid", { "data-act": "repostart", "data-i": data.repository.repositoryKey, title: `Starts Rerail in every checkout and worktree of ${repo}.` }, "Start Rerail"))));
}

/**
 * Day one, before the first session: which rule packs are added, how many rules steer (a pack's rules steer from the
 * start; only the ones it starts off are off), and what Rerail adds to a linter, in set-up's own words. The welcome
 * shows only where Rerail is started: a repository set up but never started reads as stopped.
 */
function welcomeBanner(repo) {
  const running = rulesArray().filter(isRunning), log = running.filter(isLog).length, steer = running.length - log;
  const names = [...data.attachedPacks.keys()].map(id => packById(id)?.name).filter(Boolean).sort((a, b) => a.localeCompare(b));
  return h("div.welcome", { role: "status" }, icon("wave", 22), h("div", null,
    h("b", null, `Hi! Rerail is on in ${repo}: ${fmt(names.length)} ${plural(names.length, "rule pack")}, ${fmt(steer)} ${plural(steer, "rule")} steering${log ? `, ${fmt(log)} on Log only` : ""}.`),
    names.length ? h("p", null, `${andList(names)}. Each pack starts some of its rules on and the rest off; turn more on from its page.`) : null,
    h("p", null, "Your linter checks code after it’s written; Rerail tells your coding agent as it writes, so problems are fixed in the same turn."),
    h("p.acts", null, h("button.btn.sm", { "data-act": "nav", "data-i": "packs" }, "See your rule packs"))),
  h("button.x", { "data-act": "dismisswelcome", "aria-label": "Dismiss" }, "✕"));
}

function renderBanner(root, state) {
  const kind = data.loaded ? bannerFor(state, welcomeDismissed()) : null;
  const repo = repoName();
  const on = rulesArray().filter(isRunning).length;
  setContent(root, JSON.stringify([kind, repo, on, data.repository?.repositoryKey, [...data.attachedPacks.keys()], rulesArray().filter(isRunning).map(r => r.rule.mode)]), () =>
    kind === "stopped" ? stoppedBanner(repo) : kind === "welcome" ? welcomeBanner(repo) : null);
}

/* A newer Rerail: once per version, until dismissed, on a repository that is set up. */

const updateKey = latest => `rerail-update-dismissed:${latest}`;
/** The versions dismissed in this visit, which holds when storage is blocked. */
const updateDismissedHere = new Set();

function updateDismissed(latest) {
  if (updateDismissedHere.has(latest)) return true;
  try { return localStorage.getItem(updateKey(latest)) === "1"; } catch { return false; /* storage blocked */ }
}

function dismissUpdate() {
  const latest = newerRelease(data.health);
  if (latest === null) return;
  updateDismissedHere.add(latest);
  try { localStorage.setItem(updateKey(latest), "1"); } catch { /* private mode: dismissed for this visit */ }
}

/**
 * "Rerail 0.3.0 just launched.", from the newest version the daemon reports (`/healthz.update`), with how to update:
 * the command to paste into a coding agent's session and its Copy, and Details in Settings, which goes to About Rerail
 * and its release notes. It sits below the banners that ask for something, since it doesn't.
 */
function renderUpdate(root) {
  const latest = data.loaded && isSetUp() ? newerRelease(data.health) : null;
  const show = latest !== null && !updateDismissed(latest);
  setContent(root, show ? latest : "", () => (!show ? null : h("div.welcome", { role: "status" }, icon("rocket", 22), h("div", null,
    h("b", null, `Rerail ${latest} just launched.`),
    h("p", null, "To update, paste ", h("code", null, AGENT_UPDATE_COMMAND), " into a session with your coding agent. It updates your rule packs too, and backs up your rules and history first."),
    h("p.acts", null, updateCopyButton("button.btn.sm", "Copy update command"), updateDetailsLink())),
  h("button.x", { "data-act": "dismissupdate", "aria-label": "Dismiss" }, "✕"))));
}

/** The worktree's `.rerail/rules` files left out, each with why; they run nothing until fixed. Not dismissible. */
function renderLeftOut(root) {
  const leftOut = data.loaded ? worktreeLeftOut(data.packsSkipped, data.checkout?.checkoutPath) : [];
  const n = leftOut.length;
  setContent(root, JSON.stringify(leftOut), () => (n === 0 ? null : h("div.welcome.warn", { role: "status" }, icon("steered", 22), h("div", null,
    h("b", null, `${fmt(n)} ${plural(n, "file")} in this worktree's .rerail/rules ${n === 1 ? "was" : "were"} left out, so ${n === 1 ? "its rule does" : "their rules do"} not run.`),
    ...leftOut.map(({ file, reason }) => h("p", null, h("span.mono", null, file), `: ${reason}`)),
    h("p", null, "Fix the file and this updates on its own. ", h("span.mono", null, "rerail packs list"), " in the worktree says the same.")))));
}

/** The rules created by you that wait for your approval, by title: the ones `/rerail-create` added in Log. */
export const yourRulesToApprove = () => (data.loaded ? rulesArray().filter(awaitingYourApproval) : [])
  .sort((a, b) => titleOf(a).localeCompare(titleOf(b), undefined, { sensitivity: "base" }) || a.rule.id.localeCompare(b.rule.id));

/**
 * The rules your coding agent added that wait for your approval (`/rerail-create` adds the rules it suggests this way):
 * how many, what they caught in Log that the agent was not told, and one button to Created by you, where each is opened
 * and read in full before it is approved from its sheet. Not dismissible: a rule left here keeps running in Log.
 */
function renderYours(root) {
  const rules = yourRulesToApprove(), n = rules.length, one = n === 1;
  const caught = rules.reduce((sum, stored) => sum + allFires(stored.rule.id), 0);
  setContent(root, JSON.stringify([rules.map(stored => stored.rule.id), caught]), () => (n === 0 ? null : h("div.welcome.warn.yours", { role: "status" }, pendingIcon(), h("div", null,
    h("b", null, one ? "Your coding agent added a rule. It’s on Log only until you approve it."
      : `Your coding agent added ${fmt(n)} rules. They’re on Log only until you approve them.`),
    h("p", null, caught === 0 ? "Nothing caught in Log yet. "
      : `${one ? "It" : "Together they"} caught ${fmt(caught)} ${plural(caught, "violation")} in Log, which your coding agent wasn’t told about. `,
      `Open ${one ? "it" : "each one"} in Created by you to see what it checks and what it tells the coding agent, then approve it on its page.`),
    h("p.acts", null, h("button.btn.sm", { "data-act": "nav", "data-i": "created" }, "Review in Created by you"))))));
}

/** A waiting rule's item in the approval banner: its pack's page for a pack that waits, else the rule's sheet. */
function approvalItem(stored) {
  const { kind, id } = stored.approval.subject;
  const packId = stored.source.kind === "pack" ? stored.source.id : packOfRule(stored)?.id;
  return kind === "pack" && packId !== undefined
    ? { kind, label: `pack ${id}`, act: "viewpack", target: packId }
    : { kind: "rule", label: titleOf(stored), act: "detail", target: stored.rule.id };
}

/** The rules and packs here that someone else wrote and that wait for approval, one item each. */
function othersToApprove() {
  const items = new Map();
  for (const stored of data.loaded ? rulesArray() : []) {
    if (!awaitingApproval(stored) || awaitingYourApproval(stored)) continue;
    const { kind, id } = stored.approval.subject;
    if (!items.has(`${kind}\0${id}`)) items.set(`${kind}\0${id}`, approvalItem(stored));
  }
  return [...items.values()];
}

/** The approval banner's first line: how many rules and packs wait, and that they run in Log until approved. */
function waitingTitle(list) {
  const n = list.length, packs = list.filter(item => item.kind === "pack").length, rules = n - packs;
  const what = [rules && `${fmt(rules)} ${plural(rules, "rule")}`, packs && `${fmt(packs)} ${plural(packs, "pack")}`].filter(Boolean).join(" and ");
  return n === 1 ? `${what} waits for approval, so it runs in Log and steers no one.` : `${what} wait for approval, so they run in Log and steer no one.`;
}

const approvalsBanner = (list, copies) => h("div.welcome.warn", { role: "status" }, pendingIcon(), h("div", null,
  h("b", null, waitingTitle(list)),
  h("p", null, `Rerail did not write ${list.length === 1 ? "it" : "them"}. Read what each tells the coding agent, then approve it on its page. Your coding agent can audit ${list.length === 1 ? "it" : "them"} first: `,
    h("span.mono", null, "/rerail-create approve the rules"), "."),
  copies ? h("p", null, `Approving an edited copy turns off the rule it was copied from in ${repoName() || "this repo"}.`) : null,
  h("p.acts", null, list.map(item => h("button.btn.sm", { "data-act": item.act, "data-i": item.target }, item.label)))));

/**
 * The rules here whose content, or whose pack's, waits for a person's approval and that someone else wrote (a
 * worktree's own rules, packs Rerail did not publish, your edited copies of rules Rerail ships): each runs in Log until
 * then. Each item opens the sheet where it is approved, after it is read. Not dismissible.
 */
function renderApprovals(root) {
  const list = othersToApprove();
  const copies = data.loaded && rulesArray().some(stored => awaitingApproval(stored) && stored.copiedFromRuleId);
  setContent(root, JSON.stringify([list, copies]), () => (list.length === 0 ? null : approvalsBanner(list, copies)));
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

/** Before any session the range says nothing, so the activity tiles say there has been none yet. */
const NO_SESSIONS = "no sessions yet";

function renderTiles(tiles, sources, ruleCount, sessionsEver) {
  const label = rangeLabel();
  const rules = rulesArray();
  const on = rules.filter(isRunning).length;
  const { steered, total } = steeredTally(data.activity);
  setTile(tiles[0], "history?outcome=steered", steered, total, total > 0 ? label : ruleCount === 0 ? "nothing to steer yet" : sessionsEver === 0 ? NO_SESSIONS : `none ${label}`);

  const started = data.activitySessions?.started ?? 0;
  setTile(tiles[1], "sessions?filter=violations", started > 0 ? data.activitySessions.withViolations : 0, started,
    started > 0 ? label : sessionsEver > 0 ? `no sessions ${label}` : NO_SESSIONS);

  const fired = rules.filter(r => fires(r.rule.id) > 0).length;
  setTile(tiles[2], "history", fired, on, fired > 0 ? label : sessionsEver === 0 ? NO_SESSIONS : `none ${label}`);

  setTile(tiles[3], rulesOnHash(sources), on, rules.length, rulesOnSource(sources.packs.length, sources.mine));
}

/* ---------- Most violations logged ---------- */

const colhead = ruleTableHead;

function ruleRow(item) {
  return h("div.row", { "data-act": "detail", "data-i": item.id },
    h("div.rmain", null,
      h("div.rtitle", null, h("button.t", { "data-act": "detail", "data-i": item.id })),
      h("div.rside", null,
        enforcementTag(),
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
  setAttr(row.querySelector(".count"), "title", loggedTip(item.stored, item.fires));
  updateEnforcementTag(row.querySelector(".enftag"), item.stored);
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

const rulesOffCard = () => h("div.list", null, h("div.empty", null,
  h("h3", null, "Every rule is off."),
  h("p", null, "Turn rules on in a rule pack, then start a coding agent session."),
  h("p.acts", null, h("button.btn.sm.solid", { "data-act": "nav", "data-i": rulesOnHash(ruleSources()) }, "Turn rules on"))));

function emptyBlock(body) {
  const label = rangeLabel();
  if (body === "no-rules") {
    return h("div.list", null, h("div.empty", null,
      h("h3", null, "No rules added yet."),
      h("p", null, "Add a rule pack, or write a rule of your own with your coding agent."),
      h("p.acts", null,
        h("button.btn.sm.solid", { "data-act": "nav", "data-i": "packs" }, icon("plus"), "Add rule packs"),
        h("button.btn.sm", { "data-act": "nav", "data-i": "create" }, "Create a rule"))));
  }
  // The welcome banner above says what to do; this says what the space will hold, so it reads alone once dismissed.
  if (body === "no-sessions") {
    const on = rulesArray().filter(isRunning).length;
    if (on > 0) {
      return h("div.list", null, h("div.empty", null,
        h("h3", null, "No sessions yet."),
        h("p", null, "The rules that log the most violations, and your recent sessions, show here.")));
    }
    return rulesOffCard();
  }
  if (!rulesArray().some(isRunning)) return rulesOffCard();
  const { started, withViolations } = data.activitySessions ?? { started: 0, withViolations: 0 };
  // Violations from rules taken out of the repository: the sessions tile counts them, and the list of rules can't.
  const title = withViolations > 0 ? `No current rule logged violations ${label}.` : `No violations logged ${label}.`;
  const line = withViolations > 0 ? null
    : started > 0 ? `Rerail logged none in ${fmt(started)} ${plural(started, "session")} ${label}.` : `No sessions ${label}.`;
  return h("div.list", null, h("div.empty", null,
    h("h3", null, title),
    line === null ? null : h("p", null, line),
    ui.range === ALL_TIME ? null : h("p.acts", null, h("button.btn.sm", { "data-act": "range", "data-i": ALL_TIME }, "Show since install"))));
}

/* ---------- the view ---------- */

let page = null;

function build() {
  const banner = h("div.banners");
  const leftOut = h("div.banners");
  const yours = h("div.banners");
  const approvals = h("div.banners");
  const update = h("div.banners");
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
  const root = h("div.ov", null, banner, leftOut, yours, approvals, update,
    h("div.rollup", null, h("div.rollhead", null, h("h2", null, "Overview"), range), h("div.tiles", null, tiles)),
    empty, lists);
  page = { root, banner, leftOut, yours, approvals, update, range, tiles, empty, lists, topList, topNote, recent, recentNote, recentAll, recentList };
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
  renderBanner(page.banner, state);
  renderLeftOut(page.leftOut);
  renderYours(page.yours);
  renderApprovals(page.approvals);
  renderUpdate(page.update);
  syncRange(page.range);
  renderTiles(page.tiles, sources, ruleCount, sessionsEver);
  renderBody({ ruleCount, sessionsEver }, sources, repo);
}

/** Below the tiles: the most violations logged and the newest sessions, or the empty block that says what to do next. */
function renderBody({ ruleCount, sessionsEver }, sources, repo) {
  const top = topRules(rulesArray().map(stored => ({ id: stored.rule.id, title: titleOf(stored), fires: fires(stored.rule.id), stored })));
  const body = data.loaded ? overviewBody({ ruleCount, sessionsEver, topCount: top.length }) : "loading";
  page.lists.hidden = body !== "lists";
  page.empty.hidden = body === "lists" || body === "loading";
  if (body === "lists") renderLists(top, repo);
  else if (body !== "loading") {
    const on = rulesArray().filter(isRunning).length;
    setContent(page.empty, JSON.stringify([body, repo, on, ui.range, rulesOnHash(sources), data.activitySessions]), () => emptyBlock(body));
  }
}

export const actions = {
  dismisswelcome: () => { dismissWelcome(); invalidate(); },
  dismissupdate: () => { dismissUpdate(); invalidate(); },
};
