/**
 * Merge Assist (`#merge`) for the scoped checkout. Off: what turning it on does and what Rerail found (`gh`), then
 * Turn on Merge Assist. On: the status strip (branch and base, `gh`, the last scan and its problems, Scan now and Turn
 * off), the Settings fold (the conflict hook command and its timeout), then the Conflicts: open conflicts as
 * cards, the ones not quiet first (each with its short id, its summary, a failed conflict hook, both sides, the
 * evidence, and who to talk it over with), quiet ones muted, then the dismissed ones folded with who dismissed each
 * and when. A card's one action is Dismiss (not a conflict). What each shows comes from `merge-model.js`. When the
 * status cannot be read, the page says why and shows nothing to act on until a read succeeds again.
 *
 * Conflict titles, summaries, hunks, authors, branch names and links are other people's text: every one is a text
 * node (`h()` children), never markup, and a pull request link is an `href` only when `safePeerUrl` accepts it.
 *
 * Buttons the page keeps busy (Turn on, Scan now, Turn off, a card's Dismiss) are patched in place, never rebuilt for a
 * write in flight or the passing of time, so the focus stays on the button a person used.
 */
import { h, icon, setAttr, setContent, setText, syncChildren } from "../dom.js";
import { ago, failed, fmt, plural, stamp } from "../format.js";
import { data, ui, repoName } from "../state.js";
import { captureScope, inScope, writeMergeAssist } from "../poll.js";
import { invalidate, toast } from "../shell.js";
import {
  HOOK_TIMEOUT_DEFAULT, HOOK_TIMEOUT_MAX, QUIET_TEXT, TOLD_TEXT,
  branchText, conflictSections, dismissToast, githubText, hookFailure, hunkLines, quoted, readinessChecks,
  safePeerUrl, scanSummary, scannedWhat, settingsChange, settingsDraft, settingsSummary, sideHeading,
  writeError,
} from "../merge-model.js";

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

/*
 * Each write below returns early once the page shows another repository: that page's busy flags and dismissals in
 * flight are its own (`changeScope` cleared these), and the answer is not its business.
 */

async function setEnabled(enabled) {
  if (ui.mergeSwitching) return;
  const scope = captureScope();
  ui.mergeSwitching = true; invalidate();
  const status = await write(scope, "PUT", SETTINGS, { enabled }, enabled ? "Couldn’t turn on Merge Assist" : "Couldn’t turn off Merge Assist");
  if (!inScope(scope)) return;
  ui.mergeSwitching = false;
  if (status !== null) {
    toast(enabled
      ? `Merge Assist is on in ${here()}. You and your coding agent are told when your work here collides with work in flight.`
      : `Merge Assist is off in ${here()}. Rerail no longer looks for conflicts here.`);
  }
  invalidate();
}

async function scan() {
  const before = data.mergeAssist;
  if (ui.mergeScanning || before?.scanning) return;
  const scope = captureScope();
  ui.mergeScanning = true; invalidate();
  const status = await write(scope, "POST", SCAN, {}, "Couldn’t scan other branches");
  if (!inScope(scope)) return;
  ui.mergeScanning = false;
  if (status !== null) toast(scanSummary(before, status));
  invalidate();
}

/* ---------- settings ---------- */

/** The Settings form's fields: the person's edits while there are any, else the saved values (`status.settings`). */
const settingsFields = status => ui.mergeSettings ?? { ...settingsDraft(status.settings), problem: null, field: null };

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
  const scope = captureScope();
  ui.mergeSaving = true; invalidate();
  const saved = await write(scope, "PUT", SETTINGS, body, "Couldn’t save Merge Assist’s settings");
  if (!inScope(scope)) return;
  ui.mergeSaving = false;
  if (saved !== null) {
    ui.mergeSettings = null;
    toast(`Saved Merge Assist’s settings for ${here()}.`);
  }
  invalidate();
}

function fold(name) {
  if (ui.mergeOpen.has(name)) ui.mergeOpen.delete(name); else ui.mergeOpen.add(name);
  invalidate();
}

/* ---------- conflicts ---------- */

/** The open conflict `id` when it can be dismissed now: shown, and not already being dismissed; else `undefined`. */
const dismissable = id => (ui.mergeDismissing.has(id) ? undefined : data.mergeAssist?.conflicts.find(c => c.id === id && c.status === "open"));

/** Dismiss conflict `id` as not a conflict: its card leaves at once and comes back if the write fails. */
async function dismiss(id) {
  const conflict = dismissable(id);
  if (conflict === undefined) return;
  const scope = captureScope();
  const words = dismissToast(conflict);
  ui.mergeDismissing.add(id); invalidate();
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

/* ---------- pieces ---------- */

/** Text segments from `merge-model.js`: a string, or `{ code }` for a command or path. */
const segments = parts => parts.map(part => (typeof part === "string" ? part : h("code", null, part.code)));

/** A time a row shows ("4m ago"); patched on every render, so it moves on without rebuilding the row. */
const ageSlot = () => h("span.mago");

/**
 * A side's pull request as a link that opens it on GitHub, or `null` when `safePeerUrl` does not accept its URL for
 * this repository.
 */
function peerLink(side) {
  const status = data.mergeAssist;
  const href = status === null ? null : safePeerUrl(side.url, { repositoryKey: status.repositoryKey, repository: status.readiness.github.repository });
  return href === null ? null : h("a.mlink", { href, target: "_blank", rel: "noopener noreferrer", title: `Open ${side.label} on GitHub` }, side.label);
}

/* ---------- conflict cards ---------- */

/** A side's change as a diff: added lines green, removed red, headers muted. */
const hunkBlock = hunk => h("pre.mhunk", null, hunkLines(hunk).map(line => h(`span.${line.kind}`, null, line.text, "\n")));

/** Where a side's change is and when it was pushed. */
function sideWhere(side) {
  if (side.location === null && side.tip === null) return null;
  return h("div.mloc", null,
    side.location === null ? null : h("code", null, side.location),
    side.tip === null ? null : h("span", null, `${side.tip.pushed ? "pushed" : "committed"} ${stamp(Date.parse(side.tip.at))}`));
}

/** One side of a conflict: who (a link when its pull request is safe to open), what, the hunk and where. */
function sideBox(side, which) {
  const link = peerLink(side);
  const who = link === null ? sideHeading(side) : [link, side.author ? ` · ${side.author}` : null];
  return h(`div.mside.${which}`, null,
    h("span.k", null, h("span.vh", null, which === "mine" ? "Mine: " : "Theirs: "), who),
    h("p.msum", null, side.summary),
    side.hunk ? hunkBlock(side.hunk) : null,
    sideWhere(side));
}

function whyConflict(evidence) {
  if (evidence.length === 0) return null;
  return h("div.mev", null, h("div.mevb", null,
    h("span.k", null, "Why Rerail thinks this is a conflict"),
    h("ul.mwhy", null, evidence.map(line => h("li", null, line)))));
}

/** Who to settle it with, and their branch or pull request (a link when it is safe to open). */
function talkTo(theirs) {
  const where = peerLink(theirs) ?? h("span.mono", null, theirs.label);
  return h("p.mtalk", null, "Talk it over with ", theirs.author ? h("b.mauthor", null, theirs.author) : "the author", " (", where,
    "). Rerail never picks a side; dismiss it if it isn’t a conflict.");
}

/** What the other side is doing, and a failed hook. */
function conflictNotes(conflict) {
  const failure = hookFailure(conflict);
  return [
    h("p.msummary", null, conflict.summary),
    failure === null ? null : h("p.mhookfail", null, icon("logged", 14), failure),
  ];
}

function conflictCard(conflict) {
  return h(conflict.quiet ? "article.mcard.quiet" : "article.mcard", null,
    h("div.mchead", null,
      h("h3.mctitle", null, conflict.title),
      h("span.mshort", { title: `Name it in chat as #${conflict.shortId}` }, `#${conflict.shortId}`),
      h("span.mcage", null, "Open for ", ageSlot(), conflict.quiet ? null : [" · ", TOLD_TEXT])),
    conflictNotes(conflict),
    h("div.mvs", null, sideBox(conflict.mine, "mine"), sideBox(conflict.theirs, "theirs")),
    whyConflict(conflict.evidence),
    talkTo(conflict.theirs),
    h("div.macts", null,
      h("button.btn.sm.ghost", {
        "data-act": "madismiss", "data-i": conflict.id,
        "aria-label": `Dismiss ${quoted(conflict.title)}: not a conflict`, title: "Not a conflict: Rerail stops raising it",
      }, "Dismiss"),
      conflict.quiet ? h("span.mquiet", null, icon("logged"), QUIET_TEXT) : null));
}

/**
 * What a card shows, for its key: a change rebuilds the card. Its age and Dismiss's state are patched. The repository
 * `gh` names is part of it, since its pull request links depend on it (`peerLink`).
 */
const conflictContent = c => [c.quiet, c.shortId, c.title, c.mine, c.theirs, c.evidence, c.summary, c.hook, data.mergeAssist?.readiness.github.repository];

function syncCards(list, items) {
  syncChildren(list, items, c => `${c.id}\u0000${JSON.stringify(conflictContent(c))}`, conflictCard, (node, conflict) => {
    setAttr(node.querySelector('button[data-act="madismiss"]'), "disabled", ui.mergeSwitching);
    setText(node.querySelector(".mago"), ago(Date.parse(conflict.detectedAt)).replace(/ ago$/u, ""));
  });
}

/** A dismissed conflict, folded: who dismissed it and when, and whose work it was with. */
function dismissedRow(conflict) {
  const by = conflict.dismissal;
  return h("article.mrow.mclosed", null,
    h("div.mchead", null, h("span.badge", null, "Not a conflict"), h("h3.mctitle", null, conflict.title)),
    h("div.mmeta", null,
      by === null ? h("span", null, "Dismissed") : [h("span", null, "Dismissed by ", h("b.mauthor", null, by.name)), ageSlot()],
      h("span", null, `with ${sideHeading(conflict.theirs)}`)));
}

const dismissedContent = c => [c.title, c.theirs.label, c.theirs.author, c.dismissal];

/** The dismissed rows, rebuilt only when what one shows changes; each one's age is patched in place. */
function syncDismissed(list, items) {
  syncChildren(list, items, c => `${c.id}\u0000${JSON.stringify(dismissedContent(c))}`, dismissedRow, (node, conflict) => {
    const age = node.querySelector(".mago");
    if (age !== null && conflict.dismissal !== null) setText(age, ago(Date.parse(conflict.dismissal.at)));
  });
}

/* ---------- the page ---------- */

/** A folded list whose summary opens it (`mafold` with `name`), kept open across renders in `ui.mergeOpen`. */
function foldedList(name) {
  const list = h("div.list.mlist"), summary = h("summary", { "data-act": "mafold", "data-i": name });
  return { root: h("details.mfold", null, summary, h("div.in", null, list)), summary, list };
}

function buildConflicts() {
  const head = h("div.ghead"), list = h("div.mcards"), empty = h("div"), dismissed = foldedList("dismissed");
  return { root: h("section.msec.mconf", { "data-section": "conflicts" }, head, list, empty, dismissed.root), head, list, empty, dismissed };
}

/** The Settings fold: its summary (the saved values) and the form, built per saved settings (`renderSettings`). */
function buildSettings() {
  const summary = h("summary", { "data-act": "mafold", "data-i": "settings" }), form = h("div.msetform");
  return { root: h("details.mfold.msettings", { "aria-label": "Merge Assist settings" }, summary, h("div.in", null, form)), summary, form };
}

function build() {
  const head = h("div.ghead.mhead", null,
    h("h2", null, "Merge Assist", h("span.badge.mexp", null, "Experimental")),
    h("p.pitch", null, "Rerail watches the branches and pull requests in flight and tells you and your coding agent when your work collides with someone else’s, before it merges."));
  const notice = h("div"), off = h("div"), strip = h("div"), settings = buildSettings(), conflicts = buildConflicts();
  page = {
    root: h("div.mpage", null, head, notice, off, strip, settings.root, conflicts.root),
    notice, off, strip, settings, conflicts,
    // Built with the consent card and the strip; patched in place on every render.
    onButton: null, stripWhen: null, scanButton: null, offButton: null, problems: null,
  };
}

/** Why the status could not be read, or that it is being read; nothing once it is shown. */
function renderNotice(status) {
  const error = data.mergeAssistError;
  setContent(page.notice, JSON.stringify([error, status === null]), () => {
    if (error === null && status !== null) return null;
    if (error === null) return h("div.list", null, h("div.empty", null, h("p", null, "Reading Merge Assist for this checkout…")));
    return h("div.welcome.warn.mnote", { role: "alert" }, icon("steered", 14),
      h("div", null, h("b", null, "Couldn’t read Merge Assist for this checkout."),
        h("p", null, failed("The page tries again every few seconds", writeError(error)))));
  });
}

/** The consent card: what turning it on does, what Rerail found, and the one button. */
function renderOff(status) {
  const show = status !== null && !status.enabled;
  const checks = show ? readinessChecks(status.readiness) : [];
  setContent(page.off, JSON.stringify([show, checks]), () => {
    page.onButton = show ? h("button.btn.solid", { "data-act": "maon" }) : null;
    return !show ? null : h("div.list.mconsent", null,
      h("h3", null, "What turning it on does"),
      h("ul.mbul", null,
        h("li", null, "Rerail reads open pull requests through ", h("code", null, "gh"), " with your sign-in, read-only, and branches checked out in your other worktrees here. It reads remote branches only when it can’t read pull requests."),
        h("li", null, "You and your coding agent are told about each conflict, unless only lockfiles, changelogs, generated output or docs overlap. Rerail never picks a side, and never writes to your branches or to GitHub.")),
      h("h3", null, "What Rerail found"),
      h("ul.mchecks", null, checks.map(check => h(`li.${check.ok ? "mready" : "mnotready"}`, { "data-check": check.key },
        icon(check.ok ? "done" : "logged", 14),
        h("span", null, h("span.vh", null, check.ok ? "Ready: " : "Note: "), segments(check.text))))),
      h("div.mon", null,
        page.onButton,
        h("p", null, "Only a person can turn it on; a coding agent can’t through Rerail’s tools. Turn it off here or with ", h("code", null, "rerail merge-assist off"), ".")));
  });
  if (page.onButton === null) return;
  setAttr(page.onButton, "disabled", ui.mergeSwitching);
  setAttr(page.onButton, "aria-busy", ui.mergeSwitching ? "true" : null);
  setText(page.onButton, ui.mergeSwitching ? "Turning on…" : "Turn on Merge Assist");
}

function cell(label, value, attrs = null) {
  return h("div.mcell", attrs, h("span.k", null, label), h("span.v", null, value));
}

/** The strip's cells: on, branch → base, gh and the last scan. */
function stripCells(status, scan) {
  const { branch, base } = branchText(status), github = githubText(status.readiness.github);
  return h("div.mcells", null,
    cell("Status", h("span.mon-state", null, h("span.dot", { "aria-hidden": "true" }), "On")),
    cell("Branch", [h("span.mono", null, branch), h("span.marrow", { "aria-hidden": "true" }, " → "), h("span.vh", null, " into "), h("span.mono", null, base)]),
    cell("GitHub", github.ok ? github.text : h("span.mwarn-t", null, github.text)),
    cell("Last scan", scan === null ? "Not scanned yet" : [page.stripWhen, h("span.sub", null, scannedWhat(scan))]));
}

/** What the last scan could not read, folded; nothing when it read everything. Its fold is opened in place. */
function scanProblems(scan) {
  if (scan === null || scan.problems.length === 0) return null;
  const n = scan.problems.length;
  return h("details.mfold.mproblems", null,
    h("summary", { "data-act": "mafold", "data-i": "problems" }, `${fmt(n)} ${plural(n, "thing")} the last scan could not read`),
    h("div.in", null, h("ul.mbul", null, scan.problems.map(problem => h("li", null, problem)))));
}

/** Scan now and Turn off, patched in place: busy while their request is out, and both held while a switch is. */
function patchStripButtons(status) {
  const scanning = ui.mergeScanning || status.scanning, switching = ui.mergeSwitching;
  setAttr(page.scanButton, "disabled", scanning || switching);
  setAttr(page.scanButton, "aria-busy", scanning ? "true" : null);
  setText(page.scanButton, scanning ? "Scanning…" : "Scan now");
  setAttr(page.offButton, "disabled", switching);
  setText(page.offButton, switching ? "Turning off…" : "Turn off");
}

/** The status strip while on: its cells, Scan now and Turn off, and the last scan's problems. */
function renderStrip(status) {
  if (status === null || !status.enabled) {
    setContent(page.strip, "off", () => null);
    Object.assign(page, { stripWhen: null, scanButton: null, offButton: null, problems: null });
    return;
  }
  const scan = status.lastScan;
  const sig = JSON.stringify([status.branch, status.baseBranch, status.readiness, scan]);
  setContent(page.strip, sig, () => {
    page.stripWhen = scan === null ? null : h("span.when");
    page.scanButton = h("button.btn.sm.solid.mscan", { "data-act": "mascan" });
    page.offButton = h("button.btn.sm.danger.quiet", { "data-act": "maoff", title: "Turn Merge Assist off for this repository" });
    page.problems = scanProblems(scan);
    return h("div.list.mstrip", null, stripCells(status, scan), h("div.mstripacts", null, page.scanButton, page.offButton), page.problems);
  });
  patchStripButtons(status);
  if (page.problems !== null) setAttr(page.problems, "open", ui.mergeOpen.has("problems"));
  if (page.stripWhen !== null) setText(page.stripWhen, ago(Date.parse(scan.at)));
}

/** A text field of the Settings form, holding `value` when built; typing keeps it in `ui.mergeSettings` (`onInput`). */
function field(spec, attrs, value) {
  const input = h(spec, { autocomplete: "off", spellcheck: "false", ...attrs });
  input.value = value;
  return input;
}

/** The form, built with `fields` (the person's edits, else the saved values); `patchSettings` keeps the rest current. */
function settingsForm(fields) {
  return [
    h("div.mfieldrow", null,
      h("label.k", { for: "ma-hook" }, "Conflict hook command"),
      field("input.mfield.mono", { id: "ma-hook", type: "text", placeholder: "[\"./scripts/on-conflict.sh\"]", "aria-describedby": "ma-hook-hint ma-settings-problem" }, fields.command),
      h("p.mnote2", { id: "ma-hook-hint" }, "A JSON array: the program, then its arguments. Rerail runs it without a shell from the checkout’s root when a conflict is detected or closes, with the conflict as JSON on stdin. Empty for none.")),
    h("div.mfieldrow", null,
      h("label.k", { for: "ma-timeout" }, "Timeout in seconds"),
      // A text field: a number field reads "" for text it can't parse, which would save the default the person never typed.
      field("input.mfield.mnum", { id: "ma-timeout", type: "text", inputmode: "numeric", "aria-describedby": "ma-timeout-hint ma-settings-problem" }, fields.timeout),
      h("p.mnote2", { id: "ma-timeout-hint" }, `1 to ${HOOK_TIMEOUT_MAX}; ${HOOK_TIMEOUT_DEFAULT} by default. A command still running then is stopped.`)),
    h("p.msetproblem", { id: "ma-settings-problem", "aria-live": "polite" }),
    h("div.macts", null, h("button.btn.sm.solid", { "data-act": "masettings" }, "Save")),
  ];
}

/** Why the last Save was refused, and which field it was about (`aria-invalid`); nothing once a field changes. */
function patchProblem(form, fields) {
  const problem = form.querySelector('p[id="ma-settings-problem"]');
  setText(problem, fields.problem ?? "");
  setAttr(problem, "hidden", fields.problem === null);
  setAttr(form.querySelector('input[id="ma-hook"]'), "aria-invalid", fields.field === "command" ? "true" : null);
  setAttr(form.querySelector('input[id="ma-timeout"]'), "aria-invalid", fields.field === "timeout" ? "true" : null);
}

/** While a save or a switch is out: the fields read-only, Save disabled and saying so. */
function patchSaving(form) {
  const busy = ui.mergeSaving || ui.mergeSwitching;
  for (const input of form.querySelectorAll("input.mfield")) setAttr(input, "readonly", busy);
  const save = form.querySelector('button[data-act="masettings"]');
  setAttr(save, "disabled", busy);
  setAttr(save, "aria-busy", ui.mergeSaving ? "true" : null);
  setText(save, ui.mergeSaving ? "Saving…" : "Save");
}

/** The problem the last Save found, and Save's busy state, patched in place. */
function patchSettings(form, fields) {
  patchProblem(form, fields);
  patchSaving(form);
}

/**
 * The Settings fold while on: the saved values in its summary, the form rebuilt only when they or the checkout change
 * (another checkout's form must not keep what was typed there).
 */
function renderSettings(status) {
  const s = page.settings, show = status !== null && status.enabled;
  setAttr(s.root, "hidden", !show);
  if (!show) return;
  const fields = settingsFields(status);
  setAttr(s.root, "open", ui.mergeOpen.has("settings"));
  setText(s.summary, settingsSummary(status.settings));
  setContent(s.form, JSON.stringify([status.repositoryKey, status.checkout, status.settings]), () => settingsForm(fields));
  patchSettings(s.form, fields);
}

const emptyBlock = empty => h("div.list", null, h("div.empty", null, h("h3", null, empty.title), empty.lines.map(line => h("p", null, line))));

function conflictsEmpty(status, dismissed) {
  const scan = status.lastScan;
  const lines = [scan === null
    ? "Rerail looks for conflicts on every scan and after your coding agent’s edits: work in open pull requests and in your other worktrees that overlaps yours."
    : `Rerail compared this branch with ${scannedWhat(scan)} ${ago(Date.parse(scan.at))} and found nothing that collides with it.`];
  lines.push("When work overlaps, it shows here with whose it is, so you can talk it over with them.");
  if (dismissed > 0) lines.push("Earlier ones are under Dismissed.");
  return { title: "No open conflicts", lines };
}

const CONFLICTS_LEAD = "Work in flight that overlaps yours. Rerail never picks a side: talk it over with the other side’s author, and dismiss one that isn’t a conflict.";

function renderConflicts(status) {
  const c = page.conflicts, on = status !== null && status.enabled;
  setAttr(c.root, "hidden", !on);
  if (!on) return;
  const { open, dismissed } = conflictSections(status, ui.mergeDismissing);
  setContent(c.head, JSON.stringify(open.length), () => [
    h("h2", null, "Open conflicts", h("span.n2", { "aria-label": `${fmt(open.length)} open` }, fmt(open.length))),
    h("p", null, CONFLICTS_LEAD),
  ]);
  setAttr(c.list, "hidden", open.length === 0);
  syncCards(c.list, open);
  const empty = open.length === 0 ? conflictsEmpty(status, dismissed.length) : null;
  setContent(c.empty, JSON.stringify(empty), () => (empty === null ? null : emptyBlock(empty)));
  setAttr(c.dismissed.root, "hidden", dismissed.length === 0);
  setAttr(c.dismissed.root, "open", ui.mergeOpen.has("dismissed"));
  setText(c.dismissed.summary, `Dismissed (${fmt(dismissed.length)})`);
  syncDismissed(c.dismissed.list, dismissed);
}

/** The conflict a link names (`#merge?conflict=<id>`, the session's line), until a status shows whether it is here. */
let reveal = null;

export function enter(route) {
  if (route.conflict !== undefined) reveal = route.conflict;
}

/** Which conflict to bring into view this render, once the status is in; a dismissed one opens its fold. */
function revealTarget(status) {
  if (reveal === null || status === null) return null;
  const found = status.enabled ? status.conflicts.find(c => c.id === reveal) : undefined;
  reveal = null;
  if (found === undefined) return null;
  if (found.status !== "open") ui.mergeOpen.add("dismissed");
  return found;
}

/** Scroll the linked conflict's card (or folded row) into view and focus it, so a screen reader starts there. */
function showConflict(conflict) {
  if (conflict === null) return;
  const list = conflict.status === "open" ? page.conflicts.list : page.conflicts.dismissed.list;
  const node = [...list.children].find(n => n.dataset.key?.startsWith(`${conflict.id}\u0000`));
  if (node === undefined) return;
  setAttr(node, "tabindex", "-1");
  node.scrollIntoView?.({ block: "start", behavior: "smooth" });
  node.focus({ preventScroll: true });
}

/** The daemon runs without Merge Assist's experiment: the page says so and what turns it on, and offers nothing else. */
function renderExperimentOff() {
  setContent(page.notice, "experimental-off", () => h("div.list", null, h("div.empty", null,
    h("h3", null, "Merge Assist is experimental and off"),
    h("p", null, "Start the Rerail daemon with ", h("code", null, "RERAIL_EXPERIMENTAL_MERGE_ASSIST=1"), " to use it, or with ",
      h("code", null, "RERAIL_EXPERIMENTAL=1"), " for every experimental feature."))));
  for (const draw of [renderOff, renderStrip, renderSettings, renderConflicts]) draw(null);
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
  renderNotice(status);
  renderOff(status);
  renderStrip(status);
  renderSettings(status);
  renderConflicts(status);
  showConflict(linked);
}
