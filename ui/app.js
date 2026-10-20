/**
 * Entry point. One `render()` reconciles every surface from the store; user actions mutate
 * `ui` (or optimistically `data`) and call it; the poll loop calls it when a fetch changed
 * something. Nothing here rebuilds the list wholesale.
 */
import { send } from "./api.js";
import { settingWrites } from "./settings-writes.js";
import { h, setAttr, setText, setContent } from "./dom.js";
import { ago, plural, shortSessionId } from "./format.js";
import { data, ui, summary, panelRuleId, titleOf, packOfRule, rulesArray, readScope, setScope, repoName, stateHere, keptPacks, reviewing } from "./state.js";
import { startPolling, refreshNow, loadSession, loadSessionFindings, loadMoreSessions, loadEarlier, showMoreActivity, loadAgents, loadPresence, loadFollowGitignore, saveFollowGitignore, loadRecommendations, changeScope } from "./poll.js";
import { attachmentWrites, packName } from "./attachments.js";
import { renderOverview } from "./render/overview.js";
import { renderRules, allGroupsFolded, sectionKeys, groupRules } from "./render/rules.js";
import { renderActivity } from "./render/activity.js";
import { renderBanners, dismissWelcome } from "./render/banners.js";
import { renderPacks } from "./render/packs.js";
import { renderSettings } from "./render/settings.js";
import { renderPanel, resetPanelScroll, askPrompt } from "./render/panel.js";
import { renderSessions, currentSession } from "./render/sessions.js";
import { jumpTo, placeTicks } from "./render/timeline.js";
import { resumeCommand } from "./sessions-model.js";

const $ = id => document.getElementById(id);
const content = $("content"), overview = $("overview"), controls = $("controls"), panel = $("panel"), banners = $("banners");
const statusEl = $("status"), statusText = $("statusText");

/* ---------- render ---------- */

let scheduled = false;
function render() {
  scheduled = false;
  const sum = summary();
  renderHeader();
  if (document.body.dataset.view !== ui.view) document.body.dataset.view = ui.view;
  const tab = ui.view === "packs" || ui.view === "review" ? "rules" : ui.view;
  for (const b of document.querySelectorAll(".topbar [data-view]")) setAttr(b, "aria-current", b.dataset.view === tab ? "page" : null);
  // Settings is the machine's; every other view needs a repository the daemon knows.
  const picking = (ui.scope === null || data.missing) && ui.view !== "settings";
  const notSetUp = !picking && ui.view === "rules" && data.repository !== null && !data.repository.setUp;
  const onRules = ui.view === "rules" && !picking && !notSetUp;
  setAttr(banners, "hidden", !onRules);
  setAttr(overview, "hidden", !onRules);
  setAttr(controls, "hidden", !onRules);
  const kind = picking ? "picker" : notSetUp ? "not-set-up" : ui.view;
  if (content.dataset.view !== kind) {
    content.replaceChildren();
    delete content.dataset.sig;
    content.dataset.view = kind;
  }
  if (picking) {
    if (ui.scope === null) renderPicker(); else renderMissing();
  } else if (notSetUp) renderNotSetUp();
  else if (onRules) {
    for (const c of document.querySelectorAll("#chips .chip")) setAttr(c, "aria-pressed", String(c.dataset.f === ui.filter));
    if ($("sort").value !== ui.sort) $("sort").value = ui.sort;
    setText($("foldAll"), allGroupsFolded() ? "Expand all groups" : "Collapse all groups");
    setAttr($("foldAll"), "hidden", sectionKeys().length === 0);
    renderBanners(banners);
    renderOverview(overview, sum);
    renderRules(content, sum);
  } else if (ui.view === "activity") renderActivity(content);
  else if (ui.view === "sessions") renderSessions(content);
  else if (ui.view === "settings") renderSettings(content);
  else renderPacks(content, ui.view);
  renderPanel(panel);
  renderStatus();
  syncUrl();
}

/** The repository the page shows, the way back to the list, and who a rule change belongs to. */
function renderHeader() {
  const picking = ui.scope === null;
  const name = repoName();
  setText($("repoId"), name);
  setAttr($("repoId"), "hidden", name === "");
  setAttr($("repoId"), "title", data.repository?.repositoryKey ?? null);
  setAttr($("changeRepo"), "hidden", picking);
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
      : list.map(pickerRow))));
}

function pickerRow(repository) {
  const started = repository.state !== "stopped";
  return h("div.row", null, h("div.rmain", null,
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

/** A repository that is not set up: no rules run; the way to set it up, or to review its packs first. */
function renderNotSetUp() {
  const name = repoName();
  setContent(content, `not-set-up|${name}|${ui.settingUp}`, () => h("div.list", { style: "margin-top:16px" },
    h("div.empty", null, h("h3", null, `Rerail is not set up in ${name}`),
      h("p", null, "No rules run here yet. Run ", h("code", null, "rerail start"), " in the repository to choose its rule packs and start Rerail, or add the rule packs recommended for it now."),
      h("p.acts", { style: "margin-top:14px" },
        h("button.btn.solid", { "data-act": "setup", disabled: ui.settingUp }, ui.settingUp ? "Setting up…" : "Set up with recommended packs"),
        " ",
        h("button.btn", { "data-act": "goreview" }, "Review rule packs")))));
}

/** Batch a render into a microtask; several mutations in one task render once, before paint. */
function invalidate() {
  if (scheduled) return;
  scheduled = true;
  queueMicrotask(() => { if (scheduled) render(); });
}

/* ---------- header status ----------
 * One indicator for the daemon and this repo: quiet while all is well, amber when the page is
 * behind or Rerail is not on here, red once the daemon has not answered for a while. */

const DOWN_AFTER_MS = 30_000;

function status() {
  if (!data.loaded && !data.disconnected) return { cls: "", text: "Connecting…", title: "" };
  const from = !data.updatedAt ? "never" : Date.now() - data.updatedAt < 60_000 ? "under a minute ago" : ago(data.updatedAt);
  if (data.disconnected) {
    if (!data.updatedAt || Date.now() - data.updatedAt > DOWN_AFTER_MS) {
      return { cls: "down", text: `Daemon not reachable · data from ${from} · new sessions are not steered`,
        title: `The daemon at ${location.origin} did not answer. Rules do not steer sessions until it is back; this page keeps retrying.` };
    }
    return { cls: "warn", text: `Rerail running · data from ${from} · reconnecting`, title: "The last refresh failed; this page keeps retrying." };
  }
  if (ui.scope === null) return { cls: "", text: "Choose a repository", title: "" };
  if (data.missing) return { cls: "warn", text: "Unknown repository", title: "The daemon has no record of this repository or checkout." };
  const name = repoName() || "this repo", state = stateHere();
  if (state === "stopped") {
    return { cls: "warn", text: `Rerail stopped in ${name} · nothing is audited · start it in Settings`, title: "Rules are not audited in this repo while Rerail is stopped." };
  }
  if (state === "not-set-up") {
    return { cls: "warn", text: `Started · Not set up in ${name} · no rules run yet`, title: "Sessions are recorded; rules run once the repository is set up." };
  }
  const version = data.health ? ` v${data.health.version}` : "";
  return { cls: "", text: "Rerail running · updating live", title: `Connected to the local daemon${version} · refreshing every 5 s` };
}

function renderStatus() {
  const s = status();
  setAttr(statusEl, "class", `status${s.cls ? ` ${s.cls}` : ""}`);
  setText(statusText, s.text);
  setAttr(statusEl, "title", s.title || null);
}
setInterval(renderStatus, 5000);

/* ---------- toast + clipboard ---------- */

let toastTimer;
function toast(message) {
  const el = $("toast");
  el.textContent = message;
  el.classList.add("show");
  clearTimeout(toastTimer);
  toastTimer = setTimeout(() => el.classList.remove("show"), 2600);
}

function copyText(text, field, message) {
  const ok = () => toast(message ?? "Copied. Paste it into a coding agent session.");
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

/** Rule settings: on or off, and suppression comments, for a rule or a pack; written as `changeOwner()`. */
const { setEnabled, setSuppressions, setPackSuppressions } = settingWrites({ toast, invalidate, owner: changeOwner });

function toggleRule(id) {
  const stored = data.rules.get(id);
  if (!stored) return;
  const enabled = !stored.enabled;
  setEnabled([id], enabled, enabled
    ? `On ${ownerWords()}. Rerail audits “${titleOf(stored)}” from the coding agent’s next turn.`
    : `Off ${ownerWords()}. Rerail stops auditing “${titleOf(stored)}”.`);
}

/** Honour or ignore linter suppression comments for one rule, from its panel. */
function suppressRule(id, suppressions) {
  const stored = data.rules.get(id);
  if (!stored) return;
  setSuppressions([id], suppressions, suppressions === "ignore"
    ? `Suppression comments ignored ${ownerWords()}. “${titleOf(stored)}” reports a violation on a line a linter comment disables.`
    : `Suppression comments honoured ${ownerWords()}. “${titleOf(stored)}” skips a line a linter comment disables.`);
}

/** Honour or ignore linter suppression comments for every rule of a pack, from its panel. */
function suppressPack(id, suppressions) {
  setPackSuppressions(id, suppressions, suppressions === "ignore"
    ? `Suppression comments ignored ${ownerWords()} in ${packName(id)}. A rule with its own setting keeps it.`
    : `Suppression comments honoured ${ownerWords()} in ${packName(id)}. A rule with its own setting keeps it.`);
}

const { addPacks, removePack, setUpRecommended, keepPack } = attachmentWrites({ toast, invalidate });

/** Start or stop every checkout of a repository (people only; it clears each checkout's own value). */
async function setRepository(key, verb) {
  try {
    const { body } = await send("POST", `/api/repository/${verb}`, { repository: key, scope: "repository" });
    toast(verb === "start" ? `Rerail started in ${body.repository.name}.` : `Rerail stopped in ${body.repository.name}. Rules and history are kept.`);
  } catch (error) {
    toast(`Could not ${verb} Rerail: ${error.message}`);
  }
  await refreshNow();
}

/* ---------- navigation ---------- */

function openRule(id, fromPack = null) {
  const changed = panelRuleId() !== id;
  if (changed) { ui.askOpen = false; resetPanelScroll(); }
  ui.backToPack = fromPack;
  ui.panel = { kind: "rule", id };
  invalidate();
  // The panel shows the rule's violations in range, which only an open panel loads.
  if (changed && data.loaded) refreshNow();
}

function closePanel() {
  ui.panel = null; ui.askOpen = false; ui.backToPack = null; ui.removeAsk = null;
  invalidate();
}

function enterView(view) {
  if (view === "sessions") selectSession(ui.selectedSession ?? currentSession()?.sessionId ?? null);
  if (view === "settings") {
    data.agentsError = null;
    loadAgents().catch(error => { data.agentsError = error.message; }).then(invalidate);
    loadPresence().then(invalidate, error => toast(`Could not read the status line setting: ${error.message}`));
    if (ui.scope !== null) loadFollowGitignore().then(invalidate, error => toast(`Could not read the Follow .gitignore setting: ${error.message}`));
  }
  if (view === "review" && ui.scope !== null && data.recommendations === null) {
    loadRecommendations().then(invalidate, error => toast(`Could not read the repo: ${error.message}`));
  }
}

function goView(view) {
  if (view === ui.view) return;
  closePanel();
  ui.view = view;
  ui.removeAsk = null;
  enterView(view);
  invalidate();
  window.scrollTo(0, 0);
  // Each view loads only its own data: the new one's now, not at the next poll.
  refreshNow();
}

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
  render();
  if (then) then();
  return error;
}

/** A Load more button: run its load, then show what it brought; a failure is a toast, the button stays. */
async function loadMore(load) {
  try { await load(); } catch (error) { toast(`Could not load more: ${error.message}`); }
  invalidate();
}

/** No selection, or one the list no longer has: pick what the view would show anyway and load it now, as a click would. */
function ensureSelection() {
  if (selecting !== null) return;
  const sess = currentSession();
  if (sess !== null && sess.sessionId !== ui.selectedSession) selectSession(sess.sessionId);
}

/* ---------- url ----------
 * The query names the repository the page shows (`?repository=<key>&checkout=<path>`) and the selected
 * session (`sessionId=<id>`); the hash names the view: `#rules`, `#activity`, `#sessions`, `#settings`,
 * `#rule-packs`, `#rule-packs?review=1` (set-up's review link) and `#rule/<id>` (a rule's panel over
 * Rules; the coding agent's steer links here). Navigation mirrors the state back with `replaceState`;
 * choosing another repository is a new history entry (`pushState`). */

const HASH = { rules: "rules", activity: "activity", sessions: "sessions", settings: "settings", packs: "rule-packs", review: "rule-packs?review=1" };

function routeFromHash() {
  const raw = location.hash.slice(1);
  if (raw.startsWith("rule/")) {
    try { return { view: "rules", rule: decodeURIComponent(raw.slice(5)) }; } catch { return { view: "rules" }; }
  }
  const [path, query] = raw.split("?");
  if (path === "rule-packs") return { view: new URLSearchParams(query).get("review") === "1" ? "review" : "packs" };
  if (path === "activity" || path === "sessions" || path === "settings") return { view: path };
  return { view: "rules" };
}

/** `sessionId` from the boot URL, applied once the first sessions list is in; `null` once consumed. */
let pendingSession = new URLSearchParams(location.search).get("sessionId");
/** A `#rule/<id>` link, checked once the first rules list is in. */
let pendingRule = null;

function syncUrl() {
  if (pendingSession !== null) return; // the deep link is still to be applied; a render must not overwrite it
  const url = new URL(location.href);
  const id = ui.view === "sessions" ? ui.selectedSession : null;
  if (id) url.searchParams.set("sessionId", id); else url.searchParams.delete("sessionId");
  const rule = ui.view === "rules" ? panelRuleId() : null;
  url.hash = rule === null ? HASH[ui.view] : `rule/${encodeURIComponent(rule).replace(/%3A/gi, ":")}`;
  if (url.href !== location.href) history.replaceState(null, "", url);
}

/** What belongs to one repository's page: its banners, filters and open panel. */
function resetRepositoryView() {
  ui.recommend = "idle";
  ui.welcomeDismissed = false;
  ui.actRule = null;
  ui.actPack = "";
  pendingRule = null;
  closePanel();
}

/** Show `repositoryKey`'s rules, or with `null` the repository picker, as a new history entry. */
function navigate(repositoryKey) {
  const url = new URL(location.href);
  for (const name of ["repository", "checkout", "sessionId", "host"]) url.searchParams.delete(name);
  if (repositoryKey !== null) url.searchParams.set("repository", repositoryKey);
  url.hash = HASH.rules;
  history.pushState(null, "", url);
  pendingSession = null;
  resetRepositoryView();
  ui.view = "rules";
  changeScope(readScope(location.search));
  invalidate();
  window.scrollTo(0, 0);
}

/** Back and forward between repositories: the URL's scope, when it differs from the page's, with its view and rule link. */
window.addEventListener("popstate", () => {
  const scope = readScope(location.search);
  if (scope?.repository === ui.scope?.repository && scope?.checkout === ui.scope?.checkout) return;
  resetRepositoryView();
  const route = routeFromHash();
  ui.view = route.view;
  changeScope(scope);
  enterView(ui.view);
  applyRoute(route);
  invalidate();
});

function applyRoute(route) {
  if (route.view !== ui.view) goView(route.view);
  if (route.rule !== undefined) { openRule(route.rule); pendingRule = data.loaded ? null : route.rule; if (data.loaded) checkRule(route.rule); }
  else if (panelRuleId() !== null) closePanel();
}

function checkRule(id) {
  if (data.rules.has(id)) return;
  if (panelRuleId() === id) closePanel();
  toast(`The rule ${id} is not in ${repoName() || "this repo"}`);
}

/** Select the deep-linked session, or the default one with a note when the daemon has no such session. */
async function applyPendingSession() {
  const id = pendingSession;
  pendingSession = null;
  const error = await selectSession(id);
  if (error === null || !error.message.endsWith("answered 404")) return;
  if (ui.selectedSession === id) selectSession(currentSession()?.sessionId ?? null);
  toast(`Session ${shortSessionId(id, new URLSearchParams(location.search).get("host"))} is not in this repo`);
}

/** After each poll: deep links apply once the first lists are in; otherwise the sessions view keeps a loaded selection. */
function onPoll() {
  invalidate();
  if (!data.loaded || ui.scope === null || data.missing) return;
  if (pendingRule !== null) { const id = pendingRule; pendingRule = null; checkRule(id); }
  if (pendingSession !== null) applyPendingSession();
  else if (ui.view === "sessions") ensureSelection();
}

/* ---------- actions ---------- */

function toggleSet(set, key) {
  if (set.has(key)) set.delete(key); else set.add(key);
}

function foldSection(key) {
  toggleSet(ui.foldedSections, key);
  for (const k of [...ui.groupOpen.keys()]) if (k.startsWith(`${key}|`)) ui.groupOpen.delete(k);
}

/** After a switch in the review. */
const keptMessage = (id, keep) => `${packName(id)} ${keep ? "kept on" : "switched off"}.`;

/** After a pack is removed: its rules that stay are the ones added to the repository on their own. */
function removedMessage(id) {
  const kept = rulesArray().filter(r => packOfRule(r)?.id === id).length;
  return kept === 0 ? `${packName(id)} removed. Its rules no longer run here and are out of Rules.`
    : `${packName(id)} removed. ${kept} of its ${kept === 1 ? "rules stays" : "rules stay"}, added on their own, under Other rules by Rerail.`;
}

const ACTIONS = {
  detail: id => openRule(id),
  packrule: id => openRule(id, ui.panel?.kind === "pack" ? ui.panel.id : null),
  panelback: () => { if (ui.backToPack) { ui.panel = { kind: "pack", id: ui.backToPack }; ui.backToPack = null; invalidate(); } else closePanel(); },
  close: () => closePanel(),
  toggle: id => toggleRule(id),
  suppress: (id, b) => suppressRule(id, b.dataset.v),
  packsuppress: (id, b) => suppressPack(id, b.dataset.v),
  gtoggle: key => {
    const all = groupRules(key), allOn = all.every(r => r.enabled), name = key.slice(key.indexOf("|") + 1);
    setEnabled(all.map(r => r.rule.id), !allOn, `${name}: ${all.length === 1 ? "its rule" : `all ${all.length} rules`} ${allOn ? "off" : "on"} ${ownerWords()}.`);
  },
  gfold: key => {
    const section = key.slice(0, key.indexOf("|"));
    const open = ui.groupOpen.get(key) ?? !ui.foldedSections.has(section);
    ui.groupOpen.set(key, !open);
    invalidate();
  },
  gfoldall: key => { foldSection(key); invalidate(); },
  foldall: () => {
    const fold = !allGroupsFolded();
    for (const key of sectionKeys()) if (ui.foldedSections.has(key) !== fold) foldSection(key);
    invalidate();
  },
  layer: key => { toggleSet(ui.hiddenSections, key); invalidate(); },
  catalog: () => goView("packs"),
  goreview: () => goView("review"),
  backrules: () => goView("rules"),
  newrule: () => { ui.panel = { kind: "create" }; invalidate(); },
  viewpack: id => { ui.panel = { kind: "pack", id }; ui.backToPack = null; invalidate(); },
  // During the review of a repository not set up yet, the panel's Add and Remove are the review's switch.
  addpack: id => (reviewing() ? keepPack(id, true, keptMessage(id, true)) : addPacks([id], `${packName(id)} added. Its rules are on.`)),
  removepack: id => {
    if (ui.removeAsk !== id) { ui.removeAsk = id; invalidate(); return; }
    if (reviewing()) keepPack(id, false, keptMessage(id, false));
    else removePack(id, () => removedMessage(id));
  },
  cancelremove: () => { ui.removeAsk = null; invalidate(); },
  catview: v => { ui.catView = v; invalidate(); },
  reviewtoggle: id => {
    const keep = !keptPacks().includes(id);
    keepPack(id, keep, keptMessage(id, keep));
  },
  recommend: async () => {
    ui.recommend = "loading"; invalidate();
    try { await loadRecommendations(); ui.recommend = "shown"; } catch (error) { ui.recommend = "idle"; toast(`Could not read the repo: ${error.message}`); }
    invalidate();
  },
  recommendno: () => { ui.recommend = "dismissed"; invalidate(); },
  addrecommended: () => {
    const ids = (data.recommendations ?? []).map(r => r.packId);
    addPacks(ids, `${ids.length} rule ${ids.length === 1 ? "pack" : "packs"} added. Their rules are on.`);
  },
  dismisswelcome: () => { dismissWelcome(); invalidate(); },
  repostart: key => setRepository(key, "start"),
  repostop: key => setRepository(key, "stop"),
  pickrepo: key => navigate(key),
  changerepo: () => navigate(null),
  setup: () => setUpRecommended(),
  owner: id => { ui.owner = id; invalidate(); },
  theme: v => applyTheme(v),
  pwidth: v => applyWidth(v === "1"),
  presence: async v => {
    const was = data.presence;
    data.presence = v; invalidate();
    try {
      await send("PUT", "/api/presentation-preferences", { repository: "", host: "", presence: v });
      toast(v === "silent" ? "The status line is hidden in new turns." : "The status line shows in new turns.");
    } catch (error) { data.presence = was; toast(`Could not save: ${error.message}`); }
    invalidate();
  },
  gitignore: v => saveFollowGitignore(v === "on", { toast, invalidate }),
  cmdcopy: text => copyText(text, null, `Copied ${text}`),
  range: id => { ui.range = Number(id); ui.recentShown = 50; invalidate(); refreshNow(); },
  more: () => loadMore(showMoreActivity),
  actrule: id => { ui.actRule = id; ui.actPack = ""; ui.recentShown = 50; goView("activity"); invalidate(); },
  actclear: () => { ui.actRule = null; invalidate(); },
  session: id => { closePanel(); goView("sessions"); selectSession(id, () => { const first = document.querySelector(".tl .corr"); if (first) jumpTo(first.dataset.key); }); },
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
  resumecopy: () => {
    const s = currentSession(); if (!s) return;
    const rc = resumeCommand(s);
    copyText(rc.cmd ?? s.sessionId, $("resumeText"), rc.cmd ? "Copied. Run it in your terminal." : "Copied the session id.");
  },
  asktoggle: id => { ui.askOpen = !ui.askOpen; openRule(id, ui.backToPack); },
  askintent: id => { ui.askIntent = id; invalidate(); },
  askcopy: () => { const id = panelRuleId(), r = id === null ? undefined : data.rules.get(id); if (r) copyText(askPrompt(r, ui.askIntent), $("askText")); },
};

document.addEventListener("click", e => {
  const b = e.target.closest("[data-act]");
  if (!b || b.disabled) { if (ui.removeAsk !== null && !e.target.closest(".rmask")) { ui.removeAsk = null; invalidate(); } return; }
  if (b.closest("summary")) e.preventDefault(); // a button in a group heading must not fold it
  // Inside the Remove question only its own buttons act; a click on its text must not open the pack behind it.
  if (ui.removeAsk !== null && e.target.closest(".rmask") && b.dataset.act !== "removepack" && b.dataset.act !== "cancelremove") return;
  if (ui.removeAsk !== null && b.dataset.act !== "removepack" && b.dataset.act !== "cancelremove") { ui.removeAsk = null; invalidate(); }
  const act = ACTIONS[b.dataset.act];
  if (act) act(b.dataset.i, b);
});
// Rows that open a pack are role=button divs: Enter and Space act like a click.
document.addEventListener("keydown", e => {
  if (e.key === "Escape" && ui.panel !== null) { closePanel(); return; }
  if ((e.key === "Enter" || e.key === " ") && e.target.matches?.("div[role=button][data-act]")) { e.preventDefault(); e.target.click(); }
});

document.querySelector(".topbar").addEventListener("click", e => { const b = e.target.closest("[data-view]"); if (b) goView(b.dataset.view); });
$("chips").addEventListener("click", e => { const c = e.target.closest(".chip"); if (c) { ui.filter = c.dataset.f; invalidate(); } });
$("q").addEventListener("input", e => { ui.query = e.target.value.trim().toLowerCase(); invalidate(); });
document.addEventListener("input", e => { if (e.target.id === "sq") { ui.sessQuery = e.target.value; invalidate(); } });
document.addEventListener("change", e => {
  const el = e.target;
  if (el.id === "showAll") { ui.showAll = el.checked; ui.evFilter = "all"; invalidate(); }
  else if (el.id === "spick") { selectSession(el.value); }
  else if (el.id === "sort") { ui.sort = el.value; invalidate(); }
  else if (el.id === "actpack") { ui.actPack = el.value; ui.actRule = null; ui.recentShown = 50; invalidate(); }
});
window.addEventListener("resize", () => { if (ui.view === "sessions") placeTicks($("rail"), $("tl")); });
window.addEventListener("hashchange", () => applyRoute(routeFromHash()));
window.addEventListener("scroll", () => { const s = window.scrollY > 4 ? "1" : ""; if (document.body.dataset.scrolled !== s) document.body.dataset.scrolled = s; }, { passive: true });

/* ---------- theme and panel width ---------- */

function applyTheme(t) {
  ui.theme = t;
  if (t) document.documentElement.dataset.theme = t; else delete document.documentElement.dataset.theme;
  try { localStorage.setItem("rerail-ui-theme", t); } catch { /* private mode: the choice lasts the session */ }
  invalidate();
}

function applyWidth(wide) {
  ui.wide = wide;
  if (wide) document.documentElement.dataset.wide = "1"; else delete document.documentElement.dataset.wide;
  try { localStorage.setItem("rerail-panel-wide", wide ? "1" : "0"); } catch { /* private mode */ }
  invalidate();
}

try {
  ui.theme = localStorage.getItem("rerail-ui-theme") || "";
  ui.wide = localStorage.getItem("rerail-panel-wide") === "1";
} catch { /* storage blocked: defaults */ }

/* ---------- boot ---------- */

/** The scope and view are fixed before the first render, so a deep link never flashes Rules. */
setScope(readScope(location.search));
const boot = pendingSession !== null ? { view: "sessions" } : routeFromHash();
ui.view = boot.view;
if (boot.rule !== undefined && ui.scope !== null) { ui.panel = { kind: "rule", id: boot.rule }; pendingRule = boot.rule; }
enterView(ui.view === "sessions" ? "" : ui.view);
invalidate();
startPolling(onPoll);
