/**
 * What Merge Assist's page says of Merge Assist itself, drawn into the page `merge.js` builds (`page`): why its status
 * cannot be read (or that it is being read), the consent card while off (what turning it on does, what Rerail found
 * through `gh`, and Turn on Merge Assist), and the status strip while on (branch and base, `gh`, the last scan and its
 * problems, Scan now and Turn off). The buttons are kept on `page` so each render patches them in place.
 *
 * Branch names are other people's text: every one is a text node (`h()` children), never markup.
 */
import { h, icon, setAttr, setContent, setText } from "../../dom.js";
import { ago, failed, fmt, plural } from "../../format.js";
import { data, ui } from "../../data/state.js";
import { branchText, githubText, readinessChecks, scannedWhat, writeError } from "../../model/merge-model.js";

/** Text segments from `merge-model.js`: a string, or `{ code }` for a command or path. */
const segments = parts => parts.map(part => (typeof part === "string" ? part : h("code", null, part.code)));

/** Why the status could not be read, or that it is being read; nothing once it is shown. */
export function renderNotice(page, status) {
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
export function renderOff(page, status) {
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
function stripCells(page, status, scan) {
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
function patchStripButtons(page, status) {
  const scanning = ui.mergeScanning || status.scanning, switching = ui.mergeSwitching;
  setAttr(page.scanButton, "disabled", scanning || switching);
  setAttr(page.scanButton, "aria-busy", scanning ? "true" : null);
  setText(page.scanButton, scanning ? "Scanning…" : "Scan now");
  setAttr(page.offButton, "disabled", switching);
  setText(page.offButton, switching ? "Turning off…" : "Turn off");
}

/** The status strip while on: its cells, Scan now and Turn off, and the last scan's problems. */
export function renderStrip(page, status) {
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
    return h("div.list.mstrip", null, stripCells(page, status, scan), h("div.mstripacts", null, page.scanButton, page.offButton), page.problems);
  });
  patchStripButtons(page, status);
  if (page.problems !== null) setAttr(page.problems, "open", ui.mergeOpen.has("problems"));
  if (page.stripWhen !== null) setText(page.stripWhen, ago(Date.parse(scan.at)));
}
