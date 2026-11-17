/**
 * Merge Assist (`#merge`) for the scoped checkout. Off: what turning it on does and what Rerail found (`gh`), then
 * Turn on Merge Assist. On: the status strip, the Settings fold, then the Conflicts. This module builds the page, holds
 * its writes (Turn on and off, Scan now, Save, Dismiss) and brings a linked conflict into view; its parts draw it:
 * `merge/status.js` the notice, the consent card and the strip, `merge/settings.js` the Settings form, and
 * `merge/conflicts.js` the conflict cards and the dismissed list. What each shows comes from `merge-model.js`. When
 * the status cannot be read, the page says why and shows nothing to act on until a read succeeds again.
 *
 * Conflict titles, summaries, hunks, authors, branch names and links are other people's text: every one is a text
 * node (`h()` children), never markup, and a pull request link is an `href` only when `safePeerUrl` accepts it.
 *
 * Buttons the page keeps busy (Turn on, Scan now, Turn off, Save) are patched in place, never rebuilt for a write in
 * flight or the passing of time. A busy button is disabled, which drops its focus, and the answer may rebuild it (or
 * Dismiss removes its card at once), so once a write is drawn the focus goes back to the control the person used, or
 * to the one that took its place (`refocusIndex`), unless the person moved it meanwhile.
 */
import { h, setContent } from "../dom.js";
import { failed } from "../format.js";
import { data, ui, repoName } from "../data/state.js";
import { captureScope, inScope } from "../data/scoped-requests.js";
import { mergeAssistMark, mergeAssistReadAfter, writeMergeAssist } from "../data/merge-assist.js";
import { invalidate, renderNow, toast } from "../shell.js";
import { dismissToast, refocusIndex, scanSummary, settingsChange, writeError } from "../model/merge-model.js";
import { renderNotice, renderOff, renderStrip } from "./merge/status.js";
import { buildSettings, renderSettings, settingsFields } from "./merge/settings.js";
import { buildConflicts, renderConflicts, showConflict } from "./merge/conflicts.js";

export const needs = ["mergeAssist"];

const SETTINGS = "/api/merge-assist/settings";
const SCAN = "/api/merge-assist/scan";
const DISMISS = "/api/merge-assist/conflicts/dismiss";

let page = null;

/* ---------- writes ---------- */

/**
 * One write to the scope fixed at the click (`writeMergeAssist`): the checkout's status it answers, or `null` when it
 * failed (a toast says why) or the page moved to another repository.
 */
async function write(scope, method, path, body, failure) {
  try {
    return await writeMergeAssist(scope, method, path, body);
  } catch (error) {
    toast(failed(failure, writeError(error)));
    return null;
  }
}

const here = () => repoName() || "this repo";

/** The page's button `act` when it has the focus, as `refocusIndex` takes it (`{ act, i, index }`), else `null`. */
function focusedControl(act) {
  const el = document.activeElement;
  if (page === null || el === null || el.dataset?.act !== act || !page.root.contains(el)) return null;
  return { act, i: el.dataset.i ?? null, index: [...page.root.querySelectorAll(`button[data-act="${act}"]`)].indexOf(el) };
}

/** Where the focus goes when nothing took `used`'s place: the Conflicts heading after a Dismiss, else the page's. */
function fallbackHeading(used) {
  return used.act === "madismiss" && !page.conflicts.root.hidden ? page.conflicts.head.querySelector("h2") : page.heading;
}

/**
 * Draw the page now and, when the focus was on `used` (`focusedControl`) and is nowhere now, put it back there or on
 * what took its place (`refocusIndex`); else on the Conflicts heading after a Dismiss, the page's heading otherwise.
 */
function drawAndRefocus(used) {
  renderNow();
  const active = document.activeElement;
  if (used === null || page === null || (active !== null && active !== document.body)) return;
  const controls = [...page.root.querySelectorAll("button[data-act]")].filter(button => !button.disabled);
  const at = refocusIndex(used, controls.map(button => ({ act: button.dataset.act, i: button.dataset.i ?? null })));
  const heading = fallbackHeading(used);
  (at >= 0 ? controls[at] : heading)?.focus();
}

/** What the toast says once Merge Assist is turned on or off. */
function switchedText(enabled) {
  return enabled
    ? `Merge Assist is on in ${here()}. You and your coding agent are told when your work here collides with work in flight.`
    : `Merge Assist is off in ${here()}. Rerail no longer looks for conflicts here.`;
}

/*
 * Each write below returns early once the page shows another repository: that page's busy flags and dismissals in
 * flight are its own (`changeScope` cleared these), and the answer is not its business.
 */

async function setEnabled(enabled) {
  if (ui.mergeSwitching) return;
  const scope = captureScope(), used = focusedControl(enabled ? "maon" : "maoff");
  ui.mergeSwitching = true; invalidate();
  const status = await write(scope, "PUT", SETTINGS, { enabled }, enabled ? "Couldn’t turn on Merge Assist" : "Couldn’t turn off Merge Assist");
  if (!inScope(scope)) return;
  ui.mergeSwitching = false;
  if (status !== null) toast(switchedText(enabled));
  drawAndRefocus(used);
}

async function scan() {
  const before = data.mergeAssist;
  if (ui.mergeScanning || before?.scanning) return;
  const scope = captureScope(), used = focusedControl("mascan");
  ui.mergeScanning = true; invalidate();
  const status = await write(scope, "POST", SCAN, {}, "Couldn’t scan other branches");
  if (!inScope(scope)) return;
  ui.mergeScanning = false;
  if (status !== null) toast(scanSummary(before, status));
  drawAndRefocus(used);
}

/* ---------- settings ---------- */

/** Change one of the Settings form's fields; a change clears the problem the last Save found. */
function editSettings(change) {
  const status = data.mergeAssist;
  if (status === null || ui.mergeSaving) return;
  ui.mergeSettings = { ...settingsFields(status), ...change, problem: null, field: null };
  invalidate();
}

/**
 * The settings change to save, or `null`: nothing to save while the status is unread or a save or switch is out, or
 * the form can't be saved, which it then says why (`settingsChange`).
 */
function settingsToSave() {
  const status = data.mergeAssist;
  if (status === null || ui.mergeSaving || ui.mergeSwitching) return null;
  const fields = settingsFields(status), change = settingsChange(fields);
  if (change.problem === undefined) return change.body;
  ui.mergeSettings = { ...fields, problem: change.problem, field: change.field };
  invalidate();
  return null;
}

/**
 * Save the Settings form: the hook command with its timeout (none when the command is empty).
 * A command that is not a JSON array of non-empty strings, or a timeout out of range, is refused here and says why.
 */
async function saveSettings() {
  const body = settingsToSave();
  if (body === null) return;
  const scope = captureScope(), used = focusedControl("masettings");
  ui.mergeSaving = true; invalidate();
  const saved = await write(scope, "PUT", SETTINGS, body, "Couldn’t save Merge Assist’s settings");
  if (!inScope(scope)) return;
  ui.mergeSaving = false;
  if (saved !== null) {
    ui.mergeSettings = null;
    toast(`Saved Merge Assist’s settings for ${here()}.`);
  }
  drawAndRefocus(used);
}

function fold(name) {
  if (ui.mergeOpen.has(name)) ui.mergeOpen.delete(name); else ui.mergeOpen.add(name);
  invalidate();
}

/* ---------- conflicts ---------- */

/** The open conflict `id` when it can be dismissed now: shown, and not already being dismissed; else `undefined`. */
const dismissable = id => (ui.mergeDismissing.has(id) ? undefined : data.mergeAssist?.conflicts.find(c => c.id === id && c.status === "open"));

/**
 * Dismiss conflict `id` as not a conflict: its card leaves at once, handing its Dismiss's focus to the next card's,
 * and comes back if the write fails.
 */
async function dismiss(id) {
  const conflict = dismissable(id);
  if (conflict === undefined) return;
  const scope = captureScope();
  const words = dismissToast(conflict);
  const used = focusedControl("madismiss");
  ui.mergeDismissing.add(id);
  drawAndRefocus(used);
  const status = await write(scope, "POST", DISMISS, { conflictId: id }, words.fail);
  if (!inScope(scope)) return;
  ui.mergeDismissing.delete(id);
  if (status !== null) toast(words.done);
  invalidate();
}

/** The Settings form's text fields, by id, and the field each one edits. */
const SETTINGS_INPUT = { "ma-hook": "command", "ma-timeout": "timeout" };

/** Typing in the Settings form, kept in `ui.mergeSettings` until Save. */
export function onInput(el) {
  if (Object.hasOwn(SETTINGS_INPUT, el.id)) editSettings({ [SETTINGS_INPUT[el.id]]: el.value });
}

export const actions = {
  maon: () => setEnabled(true),
  maoff: () => setEnabled(false),
  mascan: () => scan(),
  mafold: name => fold(name),
  masettings: () => saveSettings(),
  madismiss: id => dismiss(id),
};

/* ---------- the page ---------- */

function build() {
  const heading = h("h2", { tabindex: "-1" }, "Merge Assist", h("span.badge.mexp", null, "Experimental"));
  const head = h("div.ghead.mhead", null, heading,
    h("p.pitch", null, "Rerail watches the branches and pull requests in flight and tells you and your coding agent when your work collides with someone else’s, before it merges."));
  const notice = h("div"), off = h("div"), strip = h("div"), settings = buildSettings(), conflicts = buildConflicts();
  page = {
    root: h("div.mpage", null, head, notice, off, strip, settings.root, conflicts.root),
    heading, notice, off, strip, settings, conflicts,
    // Built with the consent card and the strip; patched in place on every render.
    onButton: null, stripWhen: null, scanButton: null, offButton: null, problems: null,
  };
}

/**
 * The conflict a link names (`#merge?conflict=<id>`, the session's line), `{ id, after }`, until a status asked for
 * after the link (`after`, `mergeAssistMark`) shows whether it is here. The status held when the link is followed may
 * be the sidebar's, half a minute old: the conflict may be new since, or dismissed.
 */
let reveal = null;

export function enter(route) {
  if (route.conflict !== undefined) reveal = { id: route.conflict, after: mergeAssistMark() };
}

/** Conflict `id` in `status` while Merge Assist is on; `undefined` when it is off or the conflict is not there. */
const conflictById = (status, id) => (status.enabled ? status.conflicts.find(c => c.id === id) : undefined);

/** Which conflict to bring into view this render, once a fresh status is in; a dismissed one opens its fold. */
function revealTarget(status) {
  if (reveal === null || status === null || !mergeAssistReadAfter(reveal.after)) return null;
  const { id } = reveal;
  const found = conflictById(status, id);
  reveal = null;
  if (found === undefined) return null;
  if (found.status !== "open") ui.mergeOpen.add("dismissed");
  return found;
}

/** The daemon runs without Merge Assist's experiment: the page says so and what turns it on, and offers nothing else. */
function renderExperimentOff() {
  setContent(page.notice, "experimental-off", () => h("div.list", null, h("div.empty", null,
    h("h3", null, "Merge Assist is experimental and off"),
    h("p", null, "Start the Rerail daemon with ", h("code", null, "RERAIL_EXPERIMENTAL_MERGE_ASSIST=1"), " to use it, or with ",
      h("code", null, "RERAIL_EXPERIMENTAL=1"), " for every experimental feature."))));
  for (const draw of [renderOff, renderStrip, renderSettings, renderConflicts]) draw(page, null);
}

export function render(root) {
  if (page === null || page.root.parentNode !== root) {
    build();
    root.replaceChildren(page.root);
  }
  if (data.mergeAssistExperiment === false) return renderExperimentOff();
  // After a failed read the page says why and offers nothing to act on: the last status may no longer hold.
  const status = data.mergeAssistError === null ? data.mergeAssist : null;
  const linked = revealTarget(status);
  renderNotice(page, status);
  renderOff(page, status);
  renderStrip(page, status);
  renderSettings(page, status);
  renderConflicts(page, status);
  showConflict(page, linked);
}
