/**
 * The top bar, over the sidebar and content and above the sheet, so nothing in it moves when the sheet opens: the
 * repository the page shows (a menu of the others when Rerail knows more than one), who a change belongs to when the
 * URL names a worktree, the status, and the theme.
 */
import { h, setAttr, setText, setContent, icon } from "../dom.js";
import { ago } from "../format.js";
import { data, ui, repoName, stateHere } from "../state.js";
import { applyTheme, invalidate } from "../shell.js";
import { repoOption } from "../render/repos.js";

const $ = id => document.getElementById(id);

/* ---------- status ----------
 * One indicator for the daemon and this repo: quiet while all is well, amber when the page is behind or Rerail is not
 * on here, red once the daemon has not answered for a while. */

const DOWN_AFTER_MS = 30_000;
const RECENT_MS = 60_000;

function status() {
  if (!data.loaded && !data.disconnected) return { cls: "", text: "Connecting…", title: "" };
  const from = !data.updatedAt ? "never" : Date.now() - data.updatedAt < RECENT_MS ? "under a minute ago" : ago(data.updatedAt);
  if (data.disconnected) {
    if (!data.updatedAt || Date.now() - data.updatedAt > DOWN_AFTER_MS) {
      return { cls: "down", text: `Daemon not reachable · data from ${from} · new sessions are not steered`,
        title: `The daemon at ${location.origin} did not answer. Rules do not steer sessions until it is back; this page keeps retrying.` };
    }
    return { cls: "warn", text: `Rerail running · data from ${from} · reconnecting`, title: "The last refresh failed; this page keeps retrying." };
  }
  if (ui.scope === null) return { cls: "", text: "Pick a repo", title: "" };
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

/** The status pill; also run on a timer, since "data from 2m ago" ages without a poll. */
export function renderStatus() {
  const s = status(), el = $("status");
  setAttr(el, "class", `status${s.cls ? ` ${s.cls}` : ""}`);
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

/* ---------- owner and theme ---------- */

function renderOwner() {
  const ctl = $("ownerCtl");
  setAttr(ctl, "hidden", ui.scope === null || data.missing || ui.scope.checkout === undefined);
  for (const b of ctl.querySelectorAll("[data-act=owner]")) setAttr(b, "aria-pressed", String(b.dataset.i === ui.owner));
}

const THEMES = ["", "light", "dark"];
const THEME_NAME = { "": "System", light: "Light", dark: "Dark" };

function renderTheme() {
  const b = $("themeBtn"), next = THEMES[(THEMES.indexOf(ui.theme) + 1) % THEMES.length];
  setAttr(b, "title", `Theme: ${THEME_NAME[ui.theme]}. Switch to ${THEME_NAME[next]}.`);
  setAttr(b, "aria-label", `Theme: ${THEME_NAME[ui.theme]}`);
  setAttr(b, "data-i", next);
}

export function renderTopbar() {
  renderRepoPick();
  renderOwner();
  renderTheme();
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
  owner: id => { ui.owner = id; invalidate(); },
  theme: theme => applyTheme(theme),
};
