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
import { afterPoll, closePanel, connectShell, invalidate, loadTheme, startRouting, syncUrl } from "./shell.js";
import { renderSheet, sheetActions } from "./sheet.js";
import { VIEWS, currentNeeds, pageKey, pageModule } from "./views/index.js";
import { closeRepoMenu, renderStatus, renderTopbar, repoMenuOpen, topbarActions, trackTopbarHeight } from "./frame/topbar.js";
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

const FRAME_ACTIONS = { ...topbarActions };

/** The actions of the region `el` is in. */
function regionActions(el) {
  if (el.closest("#panel")) return sheetActions;
  if (el.closest("#content")) return pageModule().actions ?? {};
  return FRAME_ACTIONS;
}

document.addEventListener("click", e => {
  closeRepoMenu(e.target);
  const b = e.target.closest("[data-act]");
  if (!b || b.disabled) { if (ui.removeAsk !== null && !e.target.closest(".rmask")) { ui.removeAsk = null; invalidate(); } return; }
  if (b.closest("summary")) e.preventDefault(); // a button in a group heading must not fold it
  const name = b.dataset.act;
  // Inside the Remove question only its own buttons act; a click on its text must not open the pack behind it.
  const answersRemove = name === "removepack" || name === "cancelremove";
  if (ui.removeAsk !== null && e.target.closest(".rmask") && !answersRemove) return;
  if (ui.removeAsk !== null && !answersRemove) { ui.removeAsk = null; invalidate(); }
  const act = regionActions(b)[name] ?? commonActions[name];
  if (act) act(b.dataset.i, b);
});

// Rows that open a pack are role=button divs: Enter and Space act like a click.
document.addEventListener("keydown", e => {
  if (e.key === "Escape") {
    if (closeRepoMenu(null)) return;
    if (ui.panel !== null) closePanel();
    return;
  }
  if ((e.key === "Enter" || e.key === " ") && e.target.matches?.("div[role=button][data-act]")) { e.preventDefault(); e.target.click(); }
});

document.addEventListener("input", e => { if (e.target.closest?.("#content")) pageModule().onInput?.(e.target); });
document.addEventListener("change", e => { if (e.target.closest?.("#content")) pageModule().onChange?.(e.target); });
window.addEventListener("resize", () => { pageModule().onResize?.(); if (repoMenuOpen()) invalidate(); });
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
