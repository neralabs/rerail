/**
 * The top bar, over the sidebar and content and above the sheet, so nothing in it moves when the sheet opens: the
 * sidebar toggle, the wordmark, the repository the page shows (a pill; with a chevron and a menu of the others only
 * when Rerail knows more than one), and the status. The theme is in Settings → General.
 */
import { h, setAttr, setText, setContent, icon } from "../dom.js";
import { data, ui, repoName, stateHere } from "../state.js";
import { invalidate } from "../shell.js";
import { repoOption } from "../render/repos.js";

const $ = id => document.getElementById(id);

/* ---------- status ----------
 * One indicator for the daemon and this repo: "Live" while the polls come in, "Updated
 * 3 min ago" when the page has fallen behind (a hidden tab, a slow answer), amber when Rerail is not on here or the
 * page is reconnecting, red once the daemon has not answered for a while or reports its model broken (the file at
 * fault named). The repo is in the pill beside the wordmark and the install date in Settings: the bar keeps only the
 * dot and how fresh the data is. */

const DOWN_AFTER_MS = 30_000;
/** Two missed polls (every 5 s) and the page is no longer live. */
const LIVE_MS = 12_000;
const MINUTE = 60_000;

/** "just now", "3 min ago", "2h ago", "4d ago". */
function updatedAgo(at) {
  const min = Math.max(0, Math.round((Date.now() - at) / MINUTE));
  if (min < 1) return "just now";
  if (min < 60) return `${min} min ago`;
  if (min < 1440) return `${Math.round(min / 60)}h ago`;
  return `${Math.round(min / 1440)}d ago`;
}

/* Labels stay short and leave out the repo (it is in the pill); the tooltip says what is known and what to do. Any
 * failed poll that is not the repository's 404 counts as disconnected, so the words claim only that the daemon did not
 * answer, not why. Only running sessions lose steering: the hook that starts a session may start the daemon. */
function status() {
  if (!data.loaded && !data.disconnected && !data.accessRequired) return { cls: "", text: "Connecting…", title: `Waiting for the local daemon at ${location.origin}.` };
  const rel = data.updatedAt ? updatedAgo(data.updatedAt) : "never";
  const version = data.health ? ` ${data.health.version}` : "";
  if (data.accessRequired) {
    return { cls: "down", text: "Needs the dashboard link",
      title: `${location.origin} answers only a browser that has opened its dashboard link once. Run rerail manage, or rerail daemon start --open, on this computer.` };
  }
  if (data.disconnected) {
    if (!data.updatedAt || Date.now() - data.updatedAt > DOWN_AFTER_MS) {
      const since = data.updatedAt ? ` since ${new Date(data.updatedAt).toLocaleTimeString()}` : "";
      return { cls: "down", text: "Daemon not responding",
        title: `No answer from ${location.origin}${since}. Running sessions aren't steered until it's back; a new session starts it, or run rerail daemon start.` };
    }
    return { cls: "warn", text: "Reconnecting…", title: `The last refresh failed. Showing data from ${rel}; retrying.` };
  }
  // A model whose files do not match its manifest fails every session's attach, whichever repo the page shows.
  const model = data.health?.model;
  if (model && !model.ok) {
    return { cls: "down", text: `Model: ${model.message}`,
      title: "The engine's model files do not match model/manifest.json, so no session can attach. Reinstall Rerail, or in a source checkout run git lfs install && git lfs pull and rebuild; then run rerail daemon stop and rerail daemon start." };
  }
  if (ui.scope === null) return { cls: "", dot: false, text: "No repo selected", title: "Open the dashboard from a session, or pick a repo." };
  if (data.missing) return { cls: "warn", text: "Repo not found", title: "The daemon has no record of this checkout. Run /rerail-start in a session there, or pick another repo." };
  if (data.repository?.setUp === false) {
    return { cls: "warn", text: "Not set up", title: "No rule packs chosen, so no rules run here. Run /rerail-start in a session here." };
  }
  if (stateHere() === "stopped") {
    return { cls: "warn", text: "Stopped", title: "Rerail isn't auditing or logging here. Start it from Overview or Settings, or run /rerail-start." };
  }
  if (Date.now() - data.updatedAt < LIVE_MS) return { cls: "", text: "Live", title: `Connected to daemon${version} · refreshing every 5 s.` };
  return { cls: "", text: `Updated ${rel}`, title: `Last refreshed ${new Date(data.updatedAt).toLocaleString()}. Refreshing pauses while the tab is hidden.` };
}

/** The status pill; also run on a timer, since "Updated 2 min ago" ages without a poll. */
export function renderStatus() {
  const s = status(), el = $("status");
  setAttr(el, "class", `status${s.cls ? ` ${s.cls}` : ""}`);
  setAttr(el.querySelector(".dot"), "hidden", s.dot === false);
  setText($("statusText"), s.text);
  setAttr(el, "title", s.title || null);
}

/* ---------- the repository and its menu ---------- */

let menuOpen = false;

/** The menu lists the other repositories: there is one to switch to. */
const switchable = () => data.repositories.length > (ui.scope === null ? 0 : 1);

/** The name the bar shows: the repository's, its key or path until its context loads, "Pick a repo" without one. */
function repoLabel() {
  if (ui.scope === null) return "Pick a repo";
  return repoName() || ui.scope.repository || ui.scope.checkout;
}

function renderRepoPick() {
  const el = $("repopick"), label = repoLabel(), can = switchable();
  if (!can) menuOpen = false;
  setContent(el, JSON.stringify([label, ui.scope === null, can, menuOpen, data.repository?.repositoryKey]), () => (can
    ? h(`button.repopill${ui.scope === null ? ".none" : ""}`, { "data-act": "repomenu", "aria-haspopup": "listbox", "aria-expanded": String(menuOpen), title: "Switch repo" },
      h("b", null, label), icon("caret", 11))
    : h(`span.repopill.static${ui.scope === null ? ".none" : ""}`, { title: data.repository?.repositoryKey ?? null }, h("b", null, label))));
  renderRepoMenu(el.firstChild);
}

/** The menu renders at page level (`#repolayer`), under the pill, so the bar's own surface never clips it. */
function renderRepoMenu(pill) {
  const layer = $("repolayer");
  if (!menuOpen) { setContent(layer, "", () => null); return; }
  const current = data.repository?.repositoryKey;
  setContent(layer, JSON.stringify([data.repositories, current]), () => h("div.repomenu", { role: "listbox", "aria-label": "Repos Rerail has seen" },
    data.repositories.map(repo => repoOption(repo, repo.repositoryKey === current)),
    h("p.sub", null, "One repo at a time. To set up a new repo, open a coding agent there and type ", h("span.mono", null, "/rerail-start"), ".")));
  const at = pill.getBoundingClientRect(), menu = layer.firstChild;
  menu.style.left = `${Math.round(at.left)}px`;
  menu.style.top = `${Math.round(at.bottom) + 6}px`;
}

/** Whether the repository menu is open (it follows the pill on scroll and resize). */
export const repoMenuOpen = () => menuOpen;

/** Close the repository menu after a click anywhere but its pill; `true` when it was open. */
export function closeRepoMenu(target) {
  if (!menuOpen || target?.closest?.(".repopill")) return false;
  menuOpen = false;
  invalidate();
  return true;
}

/* ---------- the sidebar toggle ----------
 * Far left of the bar, icon only, the words in its tooltip; the choice is kept on this computer and applied before
 * first paint (`index.html`), as `html[data-nav="hidden"]`. */

const NAV_KEY = "rerail-nav-hidden";
const navHidden = () => document.documentElement.dataset.nav === "hidden";

function toggleNav() {
  const hide = !navHidden();
  if (hide) document.documentElement.dataset.nav = "hidden"; else delete document.documentElement.dataset.nav;
  try { localStorage.setItem(NAV_KEY, hide ? "1" : "0"); } catch { /* private mode: the choice lasts the session */ }
  invalidate();
}

function renderNavToggle() {
  const b = $("navToggle"), hidden = navHidden();
  setAttr(b, "aria-pressed", String(!hidden));
  setAttr(b, "title", hidden ? "Show the menu" : "Hide the menu");
  setText(b.querySelector(".lbl"), hidden ? "Show menu" : "Hide menu");
}

export function renderTopbar() {
  renderNavToggle();
  renderRepoPick();
  renderStatus();
}

/** The bar's height as `--tb`: the sheet and the History views sit under it, and it wraps taller on a narrow page. */
export function trackTopbarHeight() {
  const bar = document.querySelector(".topbar");
  const measure = () => document.documentElement.style.setProperty("--tb", `${bar.offsetHeight}px`);
  measure();
  if (window.ResizeObserver) new ResizeObserver(measure).observe(bar); else window.addEventListener("resize", measure);
}

export const topbarActions = {
  repomenu: () => { menuOpen = !menuOpen; invalidate(); },
  navtoggle: () => toggleNav(),
};
