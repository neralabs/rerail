/**
 * Entry point. One `render()` reconciles every surface from the store: the top bar and sidebar (`frame/`), the page
 * in `#content` (a view from `views/`, or the page shown in its place) and the sheet in `#panel`. User actions mutate
 * `ui` (or optimistically `data`) and invalidate; the poll loop renders when a fetch changed something. Nothing here
 * rebuilds a list wholesale. A click runs the `data-act` of its region: the page's own actions inside `#content`, the
 * sheet's inside `#panel`, the frame's elsewhere; then the common ones (`actions.js`).
 */
import { setAttr } from "./dom.js";
import { ui } from "./state.js";
import { setNeeds, startPolling } from "./poll.js";
import { commonActions } from "./actions.js";
import { afterPoll, closePanel, connectShell, hideToast, invalidate, loadTheme, startRouting, syncUrl } from "./shell.js";
import { renderSheet, sheetActions } from "./sheet.js";
import { VIEWS, currentNeeds, pageKey, pageModule } from "./views/index.js";
import { closeRepoMenu, closeUpdatePop, escapeUpdatePop, renderStatus, renderTopbar, repoMenuOpen, topbarActions, trackTopbarHeight, updatePopOpen } from "./frame/topbar.js";
import { renderSidenav } from "./frame/sidenav.js";

const $ = id => document.getElementById(id);
const content = $("content"), panel = $("panel"), sidenav = $("sidenav");

/** The status ages between polls ("data from 2m ago"). */
const STATUS_EVERY_MS = 5000;

function render() {
  const key = pageKey();
  if (document.body.dataset.view !== ui.view) document.body.dataset.view = ui.view;
  setAttr(document.body, "data-page", key);
  renderTopbar();
  renderSidenav(sidenav);
  // A new page starts from an empty content column; the view rebuilds or re-attaches what it keeps.
  if (content.dataset.view !== key) {
    content.replaceChildren();
    delete content.dataset.sig;
    content.dataset.view = key;
  }
  pageModule(key).render(content);
  renderSheet(panel);
  syncUrl();
}

connectShell({
  render,
  enter: route => VIEWS[route.view].enter?.(route),
  afterPoll: () => pageModule().afterPoll?.(),
});

/* ---------- events ---------- */

const FRAME_ACTIONS = { ...topbarActions, toastclose: () => hideToast() };

/** The actions of the region `el` is in. A toast's action (Undo) may be the current page's. */
function regionActions(el) {
  if (el.closest("#panel")) return sheetActions;
  if (el.closest("#content")) return pageModule().actions ?? {};
  if (el.closest("#toast")) return { ...pageModule().actions, ...FRAME_ACTIONS };
  return FRAME_ACTIONS;
}

/** The action a click lands on, `null` when none or it is disabled. */
function clickedAction(target) {
  const b = target.closest("[data-act]");
  return b === null || b.disabled ? null : b;
}

/**
 * The Remove question while it is open: only its own buttons answer it, a click elsewhere inside it does nothing (its
 * text must not open the pack behind it), and a click anywhere else closes it. `true` when the click stops here.
 */
function removeAskTakes(target, name) {
  if (ui.removeAsk === null || name === "removepack" || name === "cancelremove") return false;
  if (target.closest(".rmask")) return true;
  ui.removeAsk = null;
  invalidate();
  return false;
}

function runAction(b) {
  const name = b.dataset.act;
  const act = regionActions(b)[name] ?? commonActions[name];
  if (b.closest("#toast")) hideToast(); // an action's own toast (Undo → "added") replaces it
  if (act) act(b.dataset.i, b);
}

document.addEventListener("click", e => {
  closeRepoMenu(e.target);
  closeUpdatePop(e.target);
  const b = clickedAction(e.target);
  if (b?.closest("summary")) e.preventDefault(); // a button in a group heading must not fold it
  if (removeAskTakes(e.target, b?.dataset.act) || b === null) return;
  runAction(b);
});

/** Esc closes what is on top: the repository menu, else the update popover, else the sheet. */
function escape() {
  if (closeRepoMenu(null) || escapeUpdatePop()) return;
  if (ui.panel !== null) closePanel();
}

/** Rows that open a pack are role=button divs: Enter and Space act like a click. */
const pressesRow = e => (e.key === "Enter" || e.key === " ") && e.target.matches?.("div[role=button][data-act]");

document.addEventListener("keydown", e => {
  if (e.key === "Escape") escape();
  else if (pressesRow(e)) { e.preventDefault(); e.target.click(); }
});

document.addEventListener("input", e => { if (e.target.closest?.("#content")) pageModule().onInput?.(e.target); });
document.addEventListener("change", e => { if (e.target.closest?.("#content")) pageModule().onChange?.(e.target); });
window.addEventListener("resize", () => { pageModule().onResize?.(); if (repoMenuOpen() || updatePopOpen()) invalidate(); });
window.addEventListener("scroll", () => {
  const s = window.scrollY > 4 ? "1" : "";
  if (document.body.dataset.scrolled !== s) document.body.dataset.scrolled = s;
  if (repoMenuOpen()) invalidate();
}, { passive: true });

/* ---------- boot ---------- */

loadTheme();
trackTopbarHeight();
setInterval(renderStatus, STATUS_EVERY_MS);
setNeeds(currentNeeds);
startRouting();
startPolling(afterPoll);
