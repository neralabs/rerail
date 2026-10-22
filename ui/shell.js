/**
 * What every view, the frame and the sheet share: rendering on demand, the toast and clipboard, the theme, rule
 * setting and rule pack writes, the sheet, and navigation (views, repositories and the URL, from `routes.js`). It
 * touches the DOM only when called, so modules that import it load in tests; `app.js` connects it to the render
 * pass and the views (`connectShell`) and starts it (`startRouting`).
 */
import { h } from "./dom.js";
import { settingWrites } from "./settings-writes.js";
import { attachmentWrites } from "./attachments.js";
import { data, ui, panelRuleId, readScope, setScope, repoName } from "./state.js";
import { refreshNow, changeScope } from "./poll.js";
import { bootRoute, parseRoute, repositoryUrl, urlFor } from "./routes.js";
import { resetPanelScroll } from "./render/panel.js";

/* ---------- the page's render pass ---------- */

const hooks = { render: () => {}, enter: () => {}, afterPoll: () => {} };

/**
 * Connect the shell to the page: `render()` reconciles every surface, `enter(route)` runs the `enter` of the view
 * `route.view` names, `afterPoll()` the current view's `afterPoll`.
 */
export function connectShell(page) {
  Object.assign(hooks, page);
}

let scheduled = false;

/** Batch a render into a microtask; several mutations in one task render once, before paint. */
export function invalidate() {
  if (scheduled) return;
  scheduled = true;
  queueMicrotask(() => { if (scheduled) renderNow(); });
}

/** Render now, for a caller that measures or scrolls what it rendered. */
export function renderNow() {
  scheduled = false;
  hooks.render();
}

/* ---------- toast, clipboard, theme ----------
 * One toast at a time (NER-2012). A success stays long enough to read, one with an action (Undo) a little longer;
 * a failure stays until it is dismissed with its ✕, so a person who looked away still learns the change did not save. */

const TOAST_MS = 6000;
const ACTION_TOAST_MS = 10000;
/** Messages that report a failure, from the writers that only have one `toast` (`attachments.js`, `settings-writes.js`, `poll.js`). */
const FAILURE = /^(Could not|Couldn’t|Couldn't|Nothing saved)\b/u;
let toastTimer;

/**
 * Show `message`. `error` (default: the message reads as a failure) keeps it until dismissed; `action`
 * (`{ label, act, id }`) adds a button that runs the common or frame action `act` with `data-i` `id`, then hides the toast.
 */
export function toast(message, { error = FAILURE.test(message), action = null } = {}) {
  const el = document.getElementById("toast");
  el.replaceChildren(...[h("span.msg", null, message),
    action === null ? null : h("button.tact", { "data-act": action.act, "data-i": action.id ?? null }, action.label),
    error ? h("button.tx", { "data-act": "toastclose", "aria-label": "Dismiss" }, "✕") : null].filter(Boolean));
  el.classList.toggle("error", error);
  el.setAttribute("aria-live", error ? "assertive" : "polite");
  el.classList.add("show");
  clearTimeout(toastTimer);
  if (!error) toastTimer = setTimeout(hideToast, action === null ? TOAST_MS : ACTION_TOAST_MS);
}

/** A failure: stays until dismissed. */
export const toastError = message => toast(message, { error: true });

export function hideToast() {
  clearTimeout(toastTimer);
  document.getElementById("toast").classList.remove("show");
}

/** Copy `text`; where the clipboard is blocked, select it in `field` (when given) and copy that. */
export function copyText(text, field, message) {
  const ok = () => toast(message ?? "Copied. Paste it into a coding agent session.");
  const manual = () => {
    if (field) { field.focus(); field.select(); }
    let done = false;
    try { done = document.execCommand("copy"); } catch { /* clipboard blocked; the field is selected */ }
    done ? ok() : toast("Select the text and copy it.");
  };
  if (navigator.clipboard?.writeText) navigator.clipboard.writeText(text).then(ok, manual); else manual();
}

const THEME_KEY = "rerail-ui-theme";

/** The theme the person chose last ("" follows the system); `index.html` applied it before first paint. */
export function loadTheme() {
  try { ui.theme = localStorage.getItem(THEME_KEY) || ""; } catch { /* storage blocked: the system's */ }
}

/** Show and keep theme `theme`: "", "light" or "dark". */
export function applyTheme(theme) {
  ui.theme = theme;
  if (theme) document.documentElement.dataset.theme = theme; else delete document.documentElement.dataset.theme;
  try { localStorage.setItem(THEME_KEY, theme); } catch { /* private mode: the choice lasts the session */ }
  invalidate();
}

/** A Load more button: run its load, then show what it brought; a failure is a toast, the button stays. */
export async function loadMore(load) {
  try { await load(); } catch (error) { toast(`Could not load more: ${error.message}`); }
  invalidate();
}

/* ---------- writes ---------- */

/**
 * Who a change belongs to: always the repository (Oct 16: no worktree-or-repository choice), also when the URL names
 * a worktree. The writers still take an owner, so a test can write as a worktree.
 */
export const changeOwner = () => "repository";
export const ownerWords = () => "for the repository";

/**
 * The page's writes: a rule's or a pack's settings, written as `changeOwner()` (`settings-writes.js`), and rule
 * packs added, removed or kept, in click order (`attachments.js`).
 */
export const writes = { ...settingWrites({ toast, invalidate, owner: changeOwner }), ...attachmentWrites({ toast, invalidate }) };

/* ---------- the sheet ---------- */

/** Open rule `id`'s sheet; `fromPack` is the pack sheet its Back returns to. */
export function openRule(id, fromPack = null) {
  const changed = panelRuleId() !== id;
  if (changed) { ui.askOpen = false; resetPanelScroll(); }
  ui.backToPack = fromPack;
  ui.panel = { kind: "rule", id };
  invalidate();
  // The sheet shows the rule's violations in range, which only an open sheet loads.
  if (changed && data.loaded) refreshNow();
}

/** Open pack `id`'s sheet. */
export function openPack(id) {
  ui.panel = { kind: "pack", id };
  ui.backToPack = null;
  invalidate();
}

export function closePanel() {
  ui.panel = null; ui.askOpen = false; ui.backToPack = null; ui.removeAsk = null;
  invalidate();
}

/* ---------- navigation ---------- */

/**
 * Show `route` (`routes.js`; `view` set): another view closes the sheet and polls for the new view's data now. The
 * view's `enter(route)` runs when it is entered, and again when the route carries a History rule or a session.
 */
export function go(route) {
  const moved = route.view !== ui.view || (route.view === "pack" && route.pack !== ui.pack);
  if (moved) {
    closePanel();
    ui.view = route.view;
    ui.pack = route.view === "pack" ? route.pack : null;
    ui.removeAsk = null;
    window.scrollTo(0, 0);
  }
  if (moved || route.filterRule !== undefined || route.session !== undefined) hooks.enter(route);
  invalidate();
  if (moved) refreshNow();
}

/** `sessionId` from the boot URL, entered once the first poll is in; `null` once consumed. */
let pendingSession = null;
/** A `#rule/<id>` link, checked once the first rules list is in. */
let pendingRule = null;

/** Mirror the page into the URL (`replaceState`): the view, its pack or rule, the open rule sheet and the selected session. */
export function syncUrl() {
  if (pendingSession !== null) return; // the deep link is still to be applied; a render must not overwrite it
  const route = { view: ui.view, pack: ui.pack ?? undefined, filterRule: ui.actRule ?? undefined, rule: panelRuleId() ?? undefined };
  const href = urlFor(location.href, route, ui.view === "sessions" ? ui.selectedSession : null);
  if (href !== location.href) history.replaceState(null, "", href);
}

/** What belongs to one repository's page: its banners, History filters and open sheet. */
function resetRepositoryView() {
  ui.welcomeDismissed = false;
  ui.actRule = null;
  pendingRule = null;
  closePanel();
}

/** Show repository `key`'s Overview, or with `null` the page that asks for a repository, as a new history entry. */
export function showRepository(key) {
  history.pushState(null, "", repositoryUrl(location.href, key));
  pendingSession = null;
  resetRepositoryView();
  ui.view = "overview";
  ui.pack = null;
  changeScope(readScope(location.search));
  invalidate();
  window.scrollTo(0, 0);
}

function checkRule(id) {
  if (data.rules.has(id)) return;
  if (panelRuleId() === id) closePanel();
  toast(`The rule ${id} is not in ${repoName() || "this repo"}`);
}

/** A route's rule link: its sheet, checked against the rules once they are in; a route without one closes a rule sheet. */
function applyRule(route) {
  if (route.rule !== undefined) {
    openRule(route.rule);
    pendingRule = data.loaded ? null : route.rule;
    if (data.loaded) checkRule(route.rule);
  } else if (panelRuleId() !== null) closePanel();
}

function applyRoute(route) {
  if (route.view !== null) go(route);
  applyRule(route);
}

/** Back and forward between repositories: the URL's scope, when it differs from the page's, with its view and rule link. */
function onPopState() {
  const scope = readScope(location.search);
  if (scope?.repository === ui.scope?.repository && scope?.checkout === ui.scope?.checkout) return;
  resetRepositoryView();
  const route = parseRoute(location.hash);
  ui.view = route.view ?? "overview";
  ui.pack = route.pack ?? null;
  changeScope(scope);
  hooks.enter({ ...route, view: ui.view });
  applyRule(route);
  invalidate();
}

/** After each poll: deep links apply once the first lists are in; then the current view's `afterPoll`. */
export function afterPoll() {
  invalidate();
  if (!data.loaded || ui.scope === null || data.missing) return;
  if (pendingRule !== null) { const id = pendingRule; pendingRule = null; checkRule(id); }
  if (pendingSession !== null) {
    const session = pendingSession;
    pendingSession = null;
    if (ui.view === "sessions") hooks.enter({ view: "sessions", session });
  } else hooks.afterPoll();
}

/** Fix the scope and view from the URL before the first render, so a deep link never flashes Overview; then follow it. */
export function startRouting() {
  setScope(readScope(location.search));
  const route = bootRoute(location.search, location.hash);
  ui.view = route.view;
  ui.pack = route.pack ?? null;
  if (route.rule !== undefined && ui.scope !== null) { ui.panel = { kind: "rule", id: route.rule }; pendingRule = route.rule; }
  // A session link is entered once the first sessions list is in, so a missing session can fall back.
  if (route.session !== undefined) pendingSession = route.session; else hooks.enter(route);
  window.addEventListener("popstate", onPopState);
  window.addEventListener("hashchange", () => applyRoute(parseRoute(location.hash)));
  invalidate();
}
