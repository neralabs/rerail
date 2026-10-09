/**
 * Entry point. One `render()` reconciles every surface from the store; user actions mutate
 * `ui` (or optimistically `data`) and call it; the poll loop calls it when a fetch changed
 * something. Nothing here rebuilds the list wholesale.
 */
import { get, send } from "./api.js";
import { h, setAttr, setText, setContent } from "./dom.js";
import { plural, secondsSince, shortSessionId } from "./format.js";
import { data, ui, summary, originGroups, prefixGroups, rulesArray, readScope, setScope } from "./state.js";
import { startPolling, refreshNow, loadSession, loadMoreFindings, changeScope, scopeTarget } from "./poll.js";
import { renderOverview } from "./render/overview.js";
import { renderRules, unmountRules } from "./render/rules.js";
import { renderFindings, unmountFindings } from "./render/findings.js";
import { renderPanel, resetPanelScroll, askPrompt } from "./render/panel.js";
import { renderSessions, currentSession } from "./render/sessions.js";
import { jumpTo, placeTicks } from "./render/timeline.js";
import { resumeCommand } from "./sessions-model.js";

const $ = id => document.getElementById(id);
const content = $("content"), overview = $("overview"), controls = $("controls"), panel = $("panel");
const freshness = $("freshness"), freshnessText = $("freshnessText");

/* ---------- render ---------- */

let scheduled = false;
function render() {
  scheduled = false;
  renderHeader();
  if (document.body.dataset.view !== ui.view) document.body.dataset.view = ui.view;
  for (const b of document.querySelectorAll(".views [data-view]")) setAttr(b, "aria-current", b.dataset.view === ui.view ? "page" : null);
  if (ui.scope === null || data.missing) {
    setAttr(overview, "hidden", true);
    setAttr(controls, "hidden", true);
    if (content.dataset.view !== "picker") { content.replaceChildren(); content.dataset.view = "picker"; delete content.dataset.sig; }
    if (ui.scope === null) renderPicker(); else renderMissing();
  } else {
    renderScoped();
  }
  renderPanel(panel);
  renderFreshness();
  syncUrl();
}

function renderScoped() {
  const sum = summary();
  const empty = data.loaded && data.rules.size === 0;
  setAttr(overview, "hidden", ui.view !== "rules" || empty);
  setAttr(controls, "hidden", ui.view !== "rules" || ui.tab !== "rules" || empty);
  for (const c of document.querySelectorAll("#chips .chip")) setAttr(c, "aria-pressed", String(c.dataset.f === ui.filter));
  if (ui.view === "rules") {
    if (content.dataset.view !== "rules") { content.replaceChildren(); content.dataset.view = "rules"; }
    renderOverview(overview, sum);
    if (empty) { unmountRules(content); unmountFindings(content); renderEmptyCatalog(); } else { $("emptyCatalog")?.remove(); renderRules(content, sum); renderFindings(content); }
  } else {
    if (content.dataset.view !== "sessions") { content.replaceChildren(); content.dataset.view = "sessions"; }
    renderSessions(content);
  }
}

const STATE_TEXT = { started: "Rerail is on", stopped: "Rerail is stopped", "not-set-up": "Started · Not set up" };

function renderHeader() {
  const picking = ui.scope === null;
  const name = data.repository?.name ?? "";
  setText($("repoId"), name);
  setAttr($("repoId"), "hidden", name === "");
  setAttr($("repoId"), "title", data.repository?.repositoryKey ?? null);
  setAttr($("changeRepo"), "hidden", picking);
  const state = data.checkout?.state ?? data.repository?.state;
  let status = "";
  if (picking) status = "Choose a repository";
  else if (data.missing) status = "Unknown repository";
  else if (state !== undefined) status = STATE_TEXT[state];
  setText($("statusText"), status);
  setAttr($("ownerCtl"), "hidden", picking || data.missing || ui.scope.checkout === undefined);
  for (const b of document.querySelectorAll("#ownerCtl [data-act=owner]")) setAttr(b, "aria-pressed", String(b.dataset.i === ui.owner));
}

/** Without a repository in the URL: every repository Rerail knows, to choose one. */
function renderPicker() {
  const list = data.repositories;
  const signature = `${data.loaded}|${list.map(r => [r.repositoryKey, r.name, r.setUp, r.state, r.checkoutCount].join(",")).join("|")}`;
  setContent(content, signature, () => h("section.group", { style: "margin-top:16px" },
    h("div.ghead", null, h("h2", null, "Repositories"), h("p", null, "Choose a repository to see and change its rules.")),
    h("div.list", null, list.length === 0
      ? h("div.empty", null, h("h3", null, data.loaded ? "No repositories yet" : "Loading…"),
        h("p", null, "Run ", h("code", null, "rerail start"), " in a repository and it shows here."))
      : h("div.rows", null, list.map(pickerRow)))));
}

function pickerRow(repository) {
  const started = repository.state !== "stopped";
  return h("div.row", null, h("div.rmain", null, h("span"),
    h("div.rtitle", null, h("button.t", { "data-act": "pickrepo", "data-i": repository.repositoryKey }, repository.name)),
    h("div.rside"),
    h("div.rmeta", null,
      h(repository.setUp ? "span.badge.on" : "span.badge", null, repository.setUp ? "Set up" : "Not set up"),
      h(started ? "span.badge.on" : "span.badge", null, started ? "Started" : "Stopped"),
      h("span.mono", null, repository.repositoryKey),
      h("span", null, `${repository.checkoutCount} ${plural(repository.checkoutCount, "checkout")}`))));
}

function renderMissing() {
  const target = ui.scope.repository ?? ui.scope.checkout;
  setContent(content, `missing|${target}`, () => h("div.list", { style: "margin-top:16px" },
    h("div.empty", null, h("h3", null, "Rerail does not know this repository"),
      h("p", null, h("span.mono", null, target), ". Run ", h("code", null, "rerail start"), " in it, or choose another repository."))));
}

/** A repository with no rules: set up with none, or not set up at all, with the way to set it up. */
function renderEmptyCatalog() {
  const notSetUp = data.repository !== null && !data.repository.setUp;
  const kind = notSetUp ? `not-set-up|${data.repository.name}` : "empty";
  let el = $("emptyCatalog");
  if (el?.dataset.kind !== kind) {
    el?.remove();
    el = h("div.list", { id: "emptyCatalog", "data-kind": kind, style: "margin-top:16px" }, notSetUp
      ? h("div.empty", null, h("h3", null, `Rerail is not set up in ${data.repository.name}`),
        h("p", null, "No rules run here yet. Run ", h("code", null, "rerail start"), " in the repository to choose its rule packs and start Rerail, or add the rule packs recommended for it now."),
        h("p", { style: "margin-top:14px" }, h("button.btn.solid", { "data-act": "setup" }, "Set up with recommended packs")))
      : h("div.empty", null, h("h3", null, "No rules in this repository yet"),
        h("p", null, "Rerail is set up here with no rules. Ask your agent to create one, for example “never commit .env files”, and it appears here.")));
    content.prepend(el);
  }
  const button = el.querySelector("[data-act=setup]");
  if (button !== null) {
    setAttr(button, "disabled", ui.settingUp);
    setText(button, ui.settingUp ? "Setting up…" : "Set up with recommended packs");
  }
}

/** Batch a render into a microtask; several mutations in one task render once, before paint. */
function invalidate() {
  if (scheduled) return;
  scheduled = true;
  queueMicrotask(() => { if (scheduled) render(); });
}

function freshnessLabel(live) {
  if (!data.loaded && !data.disconnected) return "Connecting…";
  if (live) return `Live · updated ${secondsSince(data.updatedAt)}s ago`;
  return "Disconnected · retrying";
}

function renderFreshness() {
  const live = data.loaded && !data.disconnected;
  setAttr(freshness, "class", `pill ${live ? "live" : "off"}`);
  setText(freshnessText, freshnessLabel(live));
  setAttr(freshness, "title", live ? "Connected to the local daemon · refreshing every 5s" : "The daemon did not answer; the page keeps its last data and retries with backoff");
}
setInterval(renderFreshness, 1000);

/* ---------- toast + clipboard ---------- */

let toastTimer;
function toast(message) {
  const el = $("toast");
  el.textContent = message;
  el.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove("show"), 2200);
}

function copyText(text, field, message) {
  const ok = () => toast(message ?? "Copied. Paste it into your agent session.");
  const manual = () => {
    if (field) { field.focus(); field.select(); }
    let done = false;
    try { done = document.execCommand("copy"); } catch { /* clipboard blocked; the field is selected */ }
    done ? ok() : toast("Select the text and copy it.");
  };
  if (navigator.clipboard?.writeText) navigator.clipboard.writeText(text).then(ok, manual); else manual();
}

/* ---------- writes ---------- */

/** Who a change belongs to: the worktree the URL names when the person chose it, else the repository. */
const changeOwner = () => (ui.scope?.checkout === undefined ? "repository" : ui.owner);
const ownerWords = () => (changeOwner() === "checkout" ? "in this worktree" : "for the repository");

/**
 * Switch rules on or off: optimistic, one `PUT /api/repository/settings` per folder the rules are added at,
 * owned by this worktree or the repository; reverted on error. The poll that follows reads back the sources.
 */
async function setEnabled(ids, enabled, label) {
  if (ids.length === 0) return;
  // The repository, owner and rows are fixed when the action starts: switching repository mid-save writes nothing there.
  const target = scopeTarget();
  const owner = changeOwner();
  const before = new Map();
  const byFolder = new Map();
  for (const id of ids) {
    const r = data.rules.get(id);
    if (!r) continue;
    before.set(r, r.enabled);
    r.enabled = enabled;
    for (const folder of r.folders) byFolder.set(folder, [...(byFolder.get(folder) ?? []), id]);
  }
  invalidate();
  try {
    for (const [folder, folderIds] of byFolder) {
      await send("PUT", "/api/repository/settings", {
        ...target, folder, owner, targets: folderIds.map(id => ({ kind: "rule", id })), on: enabled,
      });
    }
    toast(label);
  } catch (error) {
    for (const [r, was] of before) r.enabled = was;
    toast(`Could not save: ${error.message}`);
  }
  invalidate();
  refreshNow();
}

function toggleRule(id) {
  const stored = data.rules.get(id);
  if (!stored) return;
  const enabled = !stored.enabled;
  setEnabled([id], enabled, enabled
    ? `On ${ownerWords()}. Rerail steers on this rule from the next turn.`
    : `Off ${ownerWords()}. Rerail stops steering on this rule.`);
}

/** "Set up with recommended packs": the packs recommended for this checkout, each added at its folder. */
async function setUpRecommended() {
  if (ui.settingUp) return;
  ui.settingUp = true;
  invalidate();
  // Recommended for this repository, so added to this repository even if the person switches while the GET runs.
  const target = scopeTarget();
  try {
    const { body } = await get(`/api/rule-packs/recommendations?${new URLSearchParams(target)}`);
    const add = body.recommendations.map(({ packId, folder }) => ({ sourceKind: "pack", sourceId: packId, folder }));
    await send("PUT", "/api/repository/attachments", { ...target, add, configure: true });
    toast(add.length === 0
      ? "Set up with no rule packs: none fits this repository yet."
      : `Set up with ${add.map(({ sourceId, folder }) => (folder === "" ? sourceId : `${sourceId} in ${folder}`)).join(", ")}.`);
  } catch (error) {
    toast(`Could not set up: ${error.message}`);
  }
  ui.settingUp = false;
  invalidate();
  refreshNow();
}

/* ---------- navigation ---------- */

function openPanel(id) {
  if (!data.rules.has(id)) return;
  if (id !== ui.selectedRule) { ui.askOpen = false; resetPanelScroll(); }
  ui.selectedRule = id;
  invalidate();
}

function closePanel() {
  ui.selectedRule = null; ui.askOpen = false;
  invalidate();
}

function goView(view) {
  if (view === ui.view) return;
  closePanel();
  ui.view = view;
  if (view === "sessions") selectSession(ui.selectedSession ?? currentSession()?.sessionId ?? null);
  invalidate();
  window.scrollTo(0, 0);
}

/** The load behind the current selection, while it runs; a fallback must not pre-empt a deep link's 404 check. */
let selecting = null;

/** Resolves to the load error (`null` on success) so a caller can tell a missing session from a slow one. */
async function selectSession(id, then) {
  ui.selectedSession = id;
  ui.resumeOpen = false;
  invalidate();
  if (id === null) return null;
  const load = loadSession(id).then(() => null, e => e);
  selecting = load;
  const error = await load;
  if (selecting === load) selecting = null;
  if (error) console.warn("session load failed", error);
  render();
  if (then) then();
  return error;
}

/** No selection, or one the list no longer has: pick what the view would show anyway and load it now, as a click would. */
function ensureSelection() {
  if (selecting !== null) return;
  const sess = currentSession();
  if (sess !== null && sess.sessionId !== ui.selectedSession) selectSession(sess.sessionId);
}

/* ---------- url ----------
 * The status line links here with `?repository=<key>&checkout=<path>` (the repository the page shows),
 * `sessionId=<id>` and `#sessions`; the hash names the view and the query the selected session.
 * Navigation mirrors both back with `replaceState` so a reload or a copied link lands on the same spot
 * without piling up history entries; choosing another repository is a new entry (`pushState`). */

const viewFromHash = () => location.hash === "#sessions" ? "sessions" : "rules";

/** `sessionId` from the boot URL, applied once the first sessions list is in; `null` once consumed. */
let pendingSession = new URLSearchParams(location.search).get("sessionId");

function syncUrl() {
  if (pendingSession !== null) return; // the deep link is still to be applied; a render must not overwrite it
  const url = new URL(location.href);
  const id = ui.view === "sessions" ? ui.selectedSession : null;
  if (id) url.searchParams.set("sessionId", id); else url.searchParams.delete("sessionId");
  url.hash = ui.view;
  if (url.href !== location.href) history.replaceState(null, "", url);
}

/** Show `repositoryKey`'s rules, or with `null` the repository picker, as a new history entry. */
function navigate(repositoryKey) {
  const url = new URL(location.href);
  for (const name of ["repository", "checkout", "sessionId", "host"]) url.searchParams.delete(name);
  if (repositoryKey !== null) url.searchParams.set("repository", repositoryKey);
  url.hash = "rules";
  history.pushState(null, "", url);
  pendingSession = null;
  ui.view = "rules";
  changeScope(readScope(location.search));
  invalidate();
  window.scrollTo(0, 0);
}

/** Back and forward between repositories: the URL's scope, when it differs from the page's. */
window.addEventListener("popstate", () => {
  const scope = readScope(location.search);
  if (scope?.repository === ui.scope?.repository && scope?.checkout === ui.scope?.checkout) return;
  ui.view = viewFromHash();
  changeScope(scope);
  invalidate();
});

/** Select the deep-linked session, or the default one with a note when the daemon has no such session. */
async function applyPendingSession() {
  const id = pendingSession;
  pendingSession = null;
  const error = await selectSession(id);
  if (error === null || !error.message.endsWith("answered 404")) return;
  if (ui.selectedSession === id) selectSession(currentSession()?.sessionId ?? null);
  toast(`Session ${shortSessionId(id, new URLSearchParams(location.search).get("host"))} is not in this repository`);
}

/** After each poll: the deep link applies once the first list is in; otherwise the sessions view keeps a loaded selection. */
function onPoll() {
  invalidate();
  if (!data.loaded || ui.scope === null || data.missing) return;
  if (pendingSession !== null) applyPendingSession();
  else if (ui.view === "sessions") ensureSelection();
}

/* ---------- actions ---------- */

const ACTIONS = {
  detail: id => openPanel(id),
  close: () => closePanel(),
  toggle: id => toggleRule(id),
  groupby: id => { ui.groupBy = id; invalidate(); },
  gtoggle: id => {
    const g = prefixGroups().find(x => x.key === id) ?? { all: rulesArray().filter(r => r.rule.id.split(":")[0] === id) };
    const allOn = g.all.every(r => r.enabled);
    setEnabled(g.all.map(r => r.rule.id), !allOn, allOn ? `${id}: all ${g.all.length} rules off.` : `${id}: all ${g.all.length} rules on.`);
  },
  "presets-on": () => { const ids = originGroups().presets.filter(r => !r.enabled).map(r => r.rule.id); setEnabled(ids, true, ids.length ? `Every rule shown By Rerail is on ${ownerWords()}.` : "Every rule shown By Rerail is already on."); },
  rtab: id => { ui.tab = id; invalidate(); window.scrollTo(0, 0); },
  range: id => { ui.range = Number(id); ui.recentShown = 50; invalidate(); refreshNow(); },
  more: async () => {
    ui.recentShown += 50;
    if (ui.recentShown > data.findings.length) await loadMoreFindings();
    invalidate();
  },
  rtoggle: id => { ui.openRows.has(id) ? ui.openRows.delete(id) : ui.openRows.add(id); invalidate(); },
  session: id => { closePanel(); ui.view = "sessions"; selectSession(id, () => { const first = document.querySelector(".tl .corr"); if (first) jumpTo(first.dataset.key); }); window.scrollTo(0, 0); },
  ssel: id => { ui.evFilter = "all"; selectSession(id); },
  copyid: id => copyText(id, null, `Copied ${id}`),
  jump: id => jumpTo(id),
  etoggle: (id, b) => {
    const wasOpen = b.parentElement.dataset.open === "true";
    if (wasOpen) { ui.openEvents.delete(id); ui.closedEvents.add(id); } else { ui.openEvents.add(id); ui.closedEvents.delete(id); }
    invalidate();
  },
  foldtoggle: id => { ui.openFolds.has(id) ? ui.openFolds.delete(id) : ui.openFolds.add(id); invalidate(); },
  evfilter: id => { ui.evFilter = id; invalidate(); },
  showall: () => { ui.showAll = true; ui.evFilter = "all"; invalidate(); },
  sfilter: id => { ui.sessFilter = id; invalidate(); },
  daytoggle: id => { ui.openDays.has(id) ? ui.openDays.delete(id) : ui.openDays.add(id); invalidate(); },
  daymore: id => { ui.dayShown[id] = (ui.dayShown[id] ?? 50) + 50; invalidate(); },
  resumetoggle: () => { ui.resumeOpen = !ui.resumeOpen; invalidate(); },
  resumecopy: () => {
    const s = currentSession(); if (!s) return;
    const rc = resumeCommand(s);
    copyText(rc.cmd ?? s.sessionId, $("resumeText"), rc.cmd ? "Copied. Run it in your terminal." : "Copied the session id.");
  },
  asktoggle: id => { ui.askOpen = !ui.askOpen; openPanel(id); },
  askintent: id => { ui.askIntent = id; invalidate(); },
  askcopy: () => { const r = data.rules.get(ui.selectedRule); if (r) copyText(askPrompt(r, ui.askIntent), $("askText")); },
  pickrepo: key => navigate(key),
  changerepo: () => navigate(null),
  setup: () => setUpRecommended(),
  owner: id => { ui.owner = id; invalidate(); },
};

document.addEventListener("click", e => {
  const b = e.target.closest("[data-act]");
  if (!b || b.disabled) return;
  const act = ACTIONS[b.dataset.act];
  if (act) act(b.dataset.i, b);
});

document.querySelector(".views").addEventListener("click", e => { const b = e.target.closest("[data-view]"); if (b) goView(b.dataset.view); });
$("chips").addEventListener("click", e => { const c = e.target.closest(".chip"); if (c) { ui.filter = c.dataset.f; invalidate(); } });
$("q").addEventListener("input", e => { ui.query = e.target.value.trim().toLowerCase(); invalidate(); });
document.addEventListener("input", e => { if (e.target.id === "sq") { ui.sessQuery = e.target.value; invalidate(); } });
document.addEventListener("change", e => {
  const el = e.target;
  if (el.id === "showAll") { ui.showAll = el.checked; ui.evFilter = "all"; invalidate(); }
  else if (el.id === "spick") { selectSession(el.value); }
  else if (el.dataset?.sort) { ui.sort = el.value; invalidate(); }
});
document.addEventListener("keydown", e => { if (e.key === "Escape" && ui.selectedRule !== null) closePanel(); });
window.addEventListener("resize", () => { if (ui.view === "sessions") placeTicks($("rail"), $("tl")); });
window.addEventListener("hashchange", () => goView(viewFromHash()));

/* ---------- theme ---------- */

function applyTheme(t) {
  ui.theme = t;
  if (t) document.documentElement.dataset.theme = t; else delete document.documentElement.dataset.theme;
  for (const b of document.querySelectorAll("#themeSeg button")) setAttr(b, "aria-pressed", String(b.dataset.t === t));
  try { localStorage.setItem("rerail-ui-theme", t); } catch { /* private mode: the choice lasts the session */ }
}
$("themeSeg").addEventListener("click", e => { const b = e.target.closest("button"); if (b) applyTheme(b.dataset.t); });
try { applyTheme(localStorage.getItem("rerail-ui-theme") || ""); } catch { applyTheme(""); }

/* ---------- boot ---------- */

/** The scope and view are fixed before the first render, so a `#sessions` link never flashes the rules view. */
setScope(readScope(location.search));
ui.view = pendingSession !== null ? "sessions" : viewFromHash();
invalidate();
startPolling(onPoll);
