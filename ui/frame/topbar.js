/**
 * The top bar, over the sidebar and content and above the sheet, so nothing in it moves when the sheet opens: the
 * sidebar toggle, the wordmark, the repository the page shows (a pill; with a chevron and a menu of the others only
 * when Rerail knows more than one), and the status, with the version and, while a newer one is out, "Update available"
 * and its popover. The theme is in Settings → General.
 */
import { h, setAttr, setText, setContent, icon } from "../dom.js";
import { data, ui, repoName, stateHere } from "../state.js";
import { invalidate } from "../shell.js";
import { newerRelease } from "../settings-model.js";
import { repoOption, repoHint } from "../render/repos.js";
import { AGENT_UPDATE_COMMAND, updateCopyButton, updateDetailsLink } from "../render/command.js";

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
    return { cls: "warn", text: "Stopped", title: "No rules run here, and nothing is logged. Start it from Overview or Settings, or run /rerail-start." };
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
  renderVersion(el);
}

/**
 * The version of Rerail on this computer, "v0.2.0" in quiet text at the start of the status, whatever the status's
 * colour; it opens Settings at About Rerail, where the release notes and how to update are. While the daemon reports a
 * newer version, a small "Update available" beside it opens a popover with the command that updates Rerail, until
 * Rerail is updated.
 */
function renderVersion(statusEl) {
  const version = data.health?.version ?? null;
  if (version === null) { statusEl.querySelector(".verwrap")?.remove(); return; }
  const latest = newerRelease(data.health), el = versionSlot(statusEl), key = JSON.stringify([version, latest]);
  if (el.dataset.key === key) return;
  el.dataset.key = key;
  el.replaceChildren(...versionButtons(version, latest));
}

/** The version's place at the start of the status, made the first time there is a version. */
function versionSlot(statusEl) {
  let el = statusEl.querySelector(".verwrap");
  if (!el) { el = h("span.verwrap"); statusEl.prepend(el); }
  return el;
}

function versionButtons(version, latest) {
  const ver = h("button.ver", { "data-act": "nav", "data-i": "settings?section=about", title: `Rerail ${version}, on this computer. Open Settings to see how to update.` }, `v${version}`);
  if (latest === null) return [ver];
  return [ver, h("button.vnew", { "data-act": "updatepop", "aria-haspopup": "dialog", "aria-expanded": String(updateOpen), title: `Rerail ${latest} is available` }, "Update available")];
}

/* ---------- the update popover ----------
 * "Update available" opens a small popover under it, at page level (`#updatelayer`) like the repository menu: the
 * command to paste into a coding agent's session, Copy, and Details in Settings. A click opens it, not a hover, so it
 * stays open to reach Copy and works from the keyboard; a click outside it, Esc, or using Copy or the link closes it. */

let updateOpen = false;
/** Copy takes the focus once the popover has drawn, when it has just opened. */
let focusCopy = false;

/** The "Update available" chip, while a newer version is out. */
const updateChip = () => (newerRelease(data.health) === null ? null : document.querySelector("#status .vnew"));

function renderUpdatePop() {
  const layer = $("updatelayer"), chip = updateChip();
  if (chip === null) updateOpen = false;
  else setAttr(chip, "aria-expanded", String(updateOpen));
  if (!updateOpen) { setContent(layer, "", () => null); return; }
  const latest = newerRelease(data.health);
  setContent(layer, latest, () => updatePopover(latest));
  placeUnder(layer.firstChild, chip);
  if (focusCopy) { focusCopy = false; layer.querySelector(".btn").focus({ preventScroll: true }); }
}

const updatePopover = latest => h("div.updpop", { role: "dialog", "aria-label": `Rerail ${latest} is available` },
  h("b", null, `Rerail ${latest} is available`),
  h("div.updrow", null, h("div.updcmd", null, h("code", null, AGENT_UPDATE_COMMAND)), updateCopyButton("button.btn.sm.solid", "Copy")),
  h("p", null, "Paste it into a session with your coding agent. Your rule packs update too."),
  updateDetailsLink());

/** The popover hangs under the chip, its right edge on the chip's. */
function placeUnder(pop, chip) {
  const at = chip.getBoundingClientRect();
  pop.style.top = `${Math.round(at.bottom) + 6}px`;
  pop.style.right = `${Math.max(12, Math.round(document.documentElement.clientWidth - at.right))}px`;
}

/** Whether the update popover is open (it follows the chip on resize). */
export const updatePopOpen = () => updateOpen;

/** A click that leaves the popover open: on its chip (which toggles it), or in it but not on Copy or the link. */
function keepsUpdatePop(target) {
  if (!(target instanceof Element)) return false;
  if (target.closest("#status .vnew")) return true;
  return target.closest(".updpop") !== null && target.closest("[data-act]") === null;
}

/** Close the update popover after a click anywhere it does not stay open for; `true` when it was open. */
export function closeUpdatePop(target) {
  if (!updateOpen || keepsUpdatePop(target)) return false;
  updateOpen = false;
  invalidate();
  return true;
}

/** Esc: close the popover and give the focus back to its chip; `true` when it was open. */
export function escapeUpdatePop() {
  if (!closeUpdatePop(null)) return false;
  updateChip()?.focus();
  return true;
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
  setContent(layer, JSON.stringify([data.repositories, current]), () => h("div.repomenu", { role: "listbox", "aria-label": "Repos" },
    data.repositories.map(repo => repoOption(repo, repo.repositoryKey === current)),
    h("p.sub", null, repoHint())));
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
  renderUpdatePop();
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
  updatepop: () => { updateOpen = !updateOpen; focusCopy = updateOpen; invalidate(); },
};
