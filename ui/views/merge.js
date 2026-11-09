/**
 * Merge Assist (`#merge`) for the scoped checkout. Off: what turning it on does and what Rerail found (the agent CLI,
 * `gh`, whether Git ignores `.rerail/candidates/`, whether rules can steer here), then Turn on Merge Assist. On: the
 * status strip (branch and base, the judging CLI, `gh`, the last scan and its problems, the hourly judgment budget,
 * Scan now and Turn off), then three tabs:
 * - Conflicts: open conflicts as cards, loudest first (each with its level, short id, queued and snooze state, both
 *   sides, the dimmer's reasons as its size, the evidence, Keep mine, Take theirs, Write a new decision…, Not a
 *   conflict), logged ones muted, then the resolved and dismissed ones folded. Keep mine, Take theirs and a new
 *   decision open the decision text in the card; Confirm saves it. Shown first while any conflict is open.
 * - Decisions: Decided differently, Needs your review, Added from other branches and Decisions on this branch, with
 *   the ones marked as not a rule folded under it.
 * - Decision log: the ledger view, every entry from `.rerail/decisions/` on the base branch, this branch and live
 *   refs, grouped by question, with a status chip each, the branch it was read from, and a filter by status.
 * What each shows comes from `merge-model.js`. When the status cannot be read, the page says why and shows nothing to
 * act on until a read succeeds again.
 *
 * Decisions, conflicts, authors, branch names, pull request titles and links are other people's text: every one is a
 * text node (`h()` children), never markup, and a pull request link is an `href` only when `safePeerUrl` accepts it.
 *
 * Buttons the page keeps busy (Turn on, Scan now, Turn off, the tabs, a row's or card's actions) are patched in place,
 * never rebuilt for a write in flight or the passing of time, so the focus stays on the button a person used.
 */
import { h, icon, setAttr, setClass, setContent, setText, syncChildren } from "../dom.js";
import { ago, failed, fmt, stamp, startOfDay } from "../format.js";
import { data, ui, repoName } from "../state.js";
import { captureScope, inScope, writeMergeAssist } from "../poll.js";
import { copyText, invalidate, toast } from "../shell.js";
import {
  DECISION_MAX, KIND_TEXT, LEVEL_BADGE, LEVEL_TEXT, MERGE_TABS, QUEUED_TEXT, authorOf, branchText, budgetText, conflictSections, conflictToast,
  draftProblem, draftText, githubText, hunkLines, judgeText, levelText, levelTitle, LEDGER_STATUS_TEXT, LEDGER_STATUS_TITLE, logQuestions, logStatusCounts, logWhen, peerLabel,
  quoted, readinessChecks, resolutionText, resolveBody, safePeerUrl, scanSummary, scannedWhat, sectionsOf, sideHeading,
  snoozeText, tabOf, writeError,
} from "../merge-model.js";

export const needs = ["mergeAssist"];

const SETTINGS = "/api/merge-assist/settings";
const SCAN = "/api/merge-assist/scan";
const FINDINGS = "/api/merge-assist/findings";
const CANDIDATES = "/api/merge-assist/candidates";
const CONFLICTS = "/api/merge-assist/conflicts";

let page = null;
/** What the next render focuses: the draft's text once it opens, or the button that opened it once it closes. */
let focusNext = null;

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
 * Each write below returns early once the page shows another repository: that page's busy flags and pending writes
 * are its own (`changeScope` cleared these), and the answer is not its business.
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
      ? `Merge Assist is on in ${here()}. Corrections you give your coding agent here are saved as decisions from its next turn.`
      : `Merge Assist is off in ${here()}. Decisions from other branches no longer steer here; your files in .rerail/candidates/ stay.`);
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

const findingOf = id => data.mergeAssist?.findings.find(f => f.id === id);
const decisionOf = id => data.mergeAssist?.own.find(d => d.id === id);

/** What each finding action says once it is done, and when it fails. */
const FINDING_TEXT = {
  adopt: {
    done: f => `Adopted ${quoted(f.candidate.decision)}. It steers your coding agent here from its next turn.`,
    fail: f => `Couldn’t adopt ${quoted(f.candidate.decision)}`,
  },
  dismiss: {
    done: f => `Dismissed ${quoted(f.candidate.decision)}. It stays hidden until the decision changes.`,
    fail: f => `Couldn’t dismiss ${quoted(f.candidate.decision)}`,
  },
  undo: {
    done: f => `Removed ${quoted(f.candidate.decision)} from this checkout. It no longer steers your coding agent.`,
    fail: f => `Couldn’t remove ${quoted(f.candidate.decision)}`,
  },
};

/**
 * Adopt, dismiss or undo finding `id`. A dismissed or undone finding leaves its section at once and comes back if the
 * write fails; an adopted one stays, its buttons disabled, until the daemon says it applies.
 */
async function resolveFinding(id, action) {
  const key = `finding:${id}`, finding = findingOf(id);
  if (finding === undefined || ui.mergePending.has(key)) return;
  const scope = captureScope();
  ui.mergePending.set(key, action); invalidate();
  const status = await write(scope, "PUT", FINDINGS, { findingId: id, action }, FINDING_TEXT[action].fail(finding));
  if (!inScope(scope)) return;
  ui.mergePending.delete(key);
  if (status !== null) toast(FINDING_TEXT[action].done(finding));
  invalidate();
}

/** Not a rule (`rejected`) or Restore (`active`) for one of this branch's decisions; it moves at once. */
async function setCandidate(id, state) {
  const key = `candidate:${id}`, decision = decisionOf(id);
  if (decision === undefined || ui.mergePending.has(key)) return;
  const scope = captureScope();
  ui.mergePending.set(key, state); invalidate();
  const text = quoted(decision.decision);
  const status = await write(scope, "PUT", CANDIDATES, { candidateId: id, state },
    state === "rejected" ? `Couldn’t mark ${text} as not a rule` : `Couldn’t restore ${text}`);
  if (!inScope(scope)) return;
  ui.mergePending.delete(key);
  if (status !== null) {
    if (state === "rejected") {
      const share = status.readiness.candidatesIgnored ? "" : " Commit the change so teammates who added it drop it too.";
      toast(`Marked ${text} as not a rule. It no longer steers here.${share}`, { action: { label: "Undo", act: "marestore", id } });
    } else {
      toast(`${text} is a decision on this branch again.`);
    }
  }
  invalidate();
}

function copyMessage(id) {
  const finding = findingOf(id);
  if (finding?.message) copyText(finding.message, null, `Copied. Send it to ${authorOf(finding).name ?? "the pull request’s author"}.`);
}

function fold(name) {
  if (ui.mergeOpen.has(name)) ui.mergeOpen.delete(name); else ui.mergeOpen.add(name);
  invalidate();
}

function chooseTab(name) {
  if (!MERGE_TABS.includes(name)) return;
  ui.mergeTab = name;
  invalidate();
}

/** Show only the decision log's entries with one status, or all of them again (`all`, or the status already shown). */
function filterLog(name) {
  ui.mergeLogStatus = name === "all" || name === ui.mergeLogStatus || !(name in LEDGER_STATUS_TEXT) ? null : name;
  invalidate();
}

/* ---------- conflicts ---------- */

const conflictOf = id => data.mergeAssist?.conflicts.find(c => c.id === id);
const conflictBusy = id => ui.mergePending.has(`conflict:${id}`);
/** The button that opens each draft, focused again when the draft closes. */
const OPENER = { mine: "mamine", theirs: "matheirs", new: "manew" };

/** Keep mine, Take theirs or Write a new decision…: open the decision text in the card, one draft at a time. */
function openDraft(id, action) {
  const conflict = conflictOf(id);
  if (conflict === undefined || conflictBusy(id)) return;
  ui.mergeTab = "conflicts";
  ui.mergeDraft = { id, action, text: draftText(conflict, action) };
  focusNext = { draft: true };
  invalidate();
}

function cancelDraft() {
  const draft = ui.mergeDraft;
  if (draft === null || conflictBusy(draft.id)) return;
  ui.mergeDraft = null;
  focusNext = { id: draft.id, act: OPENER[draft.action] };
  invalidate();
}

/**
 * Resolve conflict `id` with `action` (and the decision `text`). The card stays, busy, until the daemon answers with
 * the status that shows it resolved; Not a conflict hides it at once and brings it back if the write fails. A failed
 * save keeps the draft so the person can try again.
 */
async function resolveConflict(id, action, text = "") {
  const key = `conflict:${id}`, conflict = conflictOf(id);
  if (conflict === undefined || conflictBusy(id)) return;
  if (action !== "dismiss" && draftProblem(text) !== null) return;
  const scope = captureScope();
  const words = conflictToast(conflict, action, text.trim(), data.mergeAssist.readiness.candidatesIgnored);
  ui.mergeTab = "conflicts";
  ui.mergePending.set(key, action); invalidate();
  const status = await write(scope, "PUT", CONFLICTS, resolveBody(id, action, text), words.fail);
  if (!inScope(scope)) return;
  ui.mergePending.delete(key);
  if (status !== null) {
    toast(words.done);
    if (ui.mergeDraft?.id === id) ui.mergeDraft = null;
  }
  invalidate();
}

function confirmDraft(id) {
  const draft = ui.mergeDraft;
  if (draft?.id === id) return resolveConflict(id, draft.action, draft.text);
  return undefined;
}

/** Typing in a draft: kept in `ui.mergeDraft`, so a render patches its counter and Confirm without rebuilding it. */
export function onInput(el) {
  if (ui.mergeDraft === null || !el.matches?.("textarea.mdraft")) return;
  ui.mergeDraft.text = el.value;
  invalidate();
}

export const actions = {
  maon: () => setEnabled(true),
  maoff: () => setEnabled(false),
  mascan: () => scan(),
  maadopt: id => resolveFinding(id, "adopt"),
  madismiss: id => resolveFinding(id, "dismiss"),
  maundo: id => resolveFinding(id, "undo"),
  mareject: id => setCandidate(id, "rejected"),
  marestore: id => setCandidate(id, "active"),
  macopy: id => copyMessage(id),
  mafold: name => fold(name),
  matab: name => chooseTab(name),
  malog: name => filterLog(name),
  mamine: id => openDraft(id, "mine"),
  matheirs: id => openDraft(id, "theirs"),
  manew: id => openDraft(id, "new"),
  macancel: () => cancelDraft(),
  maconfirm: id => confirmDraft(id),
  manotconflict: id => resolveConflict(id, "dismiss"),
};

/* ---------- pieces ---------- */

/** Text segments from `merge-model.js`: a string, or `{ code }` for a command or path. */
const segments = parts => parts.map(part => (typeof part === "string" ? part : h("code", null, part.code)));

const scopeChips = scope => h("span.mscope", { "aria-label": "Folders" },
  scope.length === 0 ? h("span.badge", null, "Whole repository") : scope.map(glob => h("span.badge.mono", null, glob)));

const levelBadge = level => h(`span.badge.mlevel.${level}`, { title: levelTitle(level) }, levelText(level));

/** A time a row shows ("4m ago"); `syncRows` fills it on every render, so it moves on without rebuilding the row. */
const ageSlot = () => h("span.mago");

/** A peer's pull request link when `safePeerUrl` accepts it for this repository, else `null`. */
function safeLink(url) {
  const status = data.mergeAssist;
  if (status === null) return null;
  return safePeerUrl(url, { repositoryKey: status.repositoryKey, repository: status.readiness.github.repository });
}

/**
 * Where a finding came from: the pull request (a link when it is safe to open) and its title, or the branch; then who
 * is behind it: the pull request's author as GitHub reports it, or the author a branch's decision file declares.
 */
function source(finding) {
  const { peer } = finding;
  const href = peer.kind === "pull-request" ? safeLink(peer.url) : null;
  const label = peerLabel(peer);
  const where = href === null
    ? h("span", null, peer.kind === "pull-request" ? label : ["branch ", h("span.mono", null, peer.ref)])
    : h("a.mlink", { href, target: "_blank", rel: "noopener noreferrer", title: `Open ${label} on GitHub` }, label);
  const author = authorOf(finding);
  return [
    h("span", null, "From ", where, peer.kind === "pull-request" && peer.title ? [" ", h("span.mtitle", null, `“${peer.title}”`)] : null),
    author.name === null ? null : author.declared
      ? h("span", { title: "The author its decision file names. Nothing checks it." }, "declared by ", h("b.mauthor", null, author.name))
      : h("span", { title: "The pull request’s author on GitHub." }, "by ", h("b.mauthor", null, author.name)),
    peer.trusted ? null : h("span.badge.mwarn", { title: "Rerail could not confirm that its author has write access to this repository." }, "Author not verified"),
  ];
}

const button = (spec, act, id, label, { aria = null, title = null } = {}) =>
  h(`button.btn.sm${spec}`, { "data-act": act, "data-i": id, "aria-label": aria, title }, label);

/** The evidence a finding carries: where this branch breaks it, conflicting files, and the agent's explanation. */
function evidence(finding) {
  const { violations, conflicts } = finding.evidence;
  const explanation = finding.evidence.judgment?.explanation;
  if (violations.length === 0 && conflicts.length === 0 && !explanation) return null;
  return h("div.mev", null,
    violations.length === 0 ? null : h("div.mevb", null,
      h("span.k", null, "Where this branch already breaks it"),
      h("ul.mviol", null, violations.map(v => h("li", null, h("span.mono", null, `${v.path}:${fmt(v.line)}`), " — ", v.message)))),
    conflicts.length === 0 ? null : h("div.mevb", null,
      h("span.k", null, "Git reports conflicts with this branch in"),
      h("p", null, conflicts.map((path, i) => [i === 0 ? null : ", ", h("span.mono", null, path)]))),
    explanation ? h("div.mevb", null, h("span.k", null, "Your agent’s judgment"), h("p", null, explanation)) : null);
}

/* ---------- rows ---------- */

function talkRow(finding) {
  const mine = finding.evidence.mine;
  const who = authorOf(finding).name;
  return h("article.mrow.talk", null,
    h("div.mvs", null,
      mine === null ? null : h("div.mside", null, h("span.k", null, "Your decision"), h("p.mq", null, mine.decision)),
      h("div.mside.theirs", null, h("span.k", null, who === null ? "Their decision" : `${who}’s decision`), h("p.mq", null, finding.candidate.decision))),
    h("div.mmeta", null, source(finding), levelBadge(finding.candidate.level), scopeChips(finding.candidate.scope)),
    evidence(finding),
    finding.message === null ? null : h("div.mmsg", null,
      h("span.k", null, `A message for ${who ?? "the pull request’s author"}`),
      h("p", null, finding.message)),
    h("div.macts", null,
      finding.message === null ? null : button("", "macopy", finding.id, [icon("copy"), "Copy message"], { aria: `Copy the message for ${who ?? "the pull request’s author"}` }),
      button(".ghost", "madismiss", finding.id, "Dismiss", { aria: `Dismiss ${quoted(finding.candidate.decision)}` })));
}

function reviewRow(finding) {
  return h("article.mrow", null,
    h("p.mq", null, finding.candidate.decision),
    h("div.mmeta", null, source(finding), levelBadge(finding.candidate.level), scopeChips(finding.candidate.scope)),
    h("p.mreason", null, finding.reason),
    evidence(finding),
    h("div.macts", null,
      button(".solid", "maadopt", finding.id, "Adopt", { aria: `Adopt ${quoted(finding.candidate.decision)}` }),
      button(".ghost", "madismiss", finding.id, "Dismiss", { aria: `Dismiss ${quoted(finding.candidate.decision)}` })));
}

function addedRow(finding) {
  return h("article.mrow", null,
    h("p.mq", null, finding.candidate.decision),
    h("div.mmeta", null, source(finding), levelBadge(finding.candidate.level), scopeChips(finding.candidate.scope),
      h("span", null, finding.tier === "absorb" ? "Added automatically " : "Adopted by you ", ageSlot())),
    h("p.mreason", null, finding.reason),
    h("div.macts", null,
      button("", "maundo", finding.id, "Undo", { aria: `Undo ${quoted(finding.candidate.decision)}`, title: "Stop it steering your coding agent in this checkout" })));
}

/** Whether a decision of this branch is shared yet: committed, not committed, or kept local by .gitignore. */
function shareHint(decision, ignored) {
  if (ignored) return h("span.badge.mwarn", { title: "Git ignores .rerail/candidates/ here, so teammates don’t see it." }, "Stays on this machine");
  if (!decision.committed) return h("span.badge.mwarn", { title: "Commit .rerail/candidates/ and push to share it with your teammates." }, "Not committed yet");
  return null;
}

const GENERALITY = { branch: "For this branch", repository: "Repository convention" };

function ownRow(decision, ignored) {
  return h("article.mrow", null,
    h("p.mq", null, decision.decision),
    decision.amends === null ? null : h("p.mamends", null, "Amends ", quoted(decision.amends.decision)),
    h("div.mmeta", null, levelBadge(decision.level), h("span", null, GENERALITY[decision.generality] ?? decision.generality),
      scopeChips(decision.scope), h("span", null, "Saved ", ageSlot()), shareHint(decision, ignored),
      decision.amendedBy === null ? null : h("span.badge.mwarn", { title: "A later decision on this branch amends it, so it no longer applies." }, "Amended")),
    h("div.macts", null,
      button(".danger.quiet", "mareject", decision.id, "Not a rule", {
        aria: `Mark ${quoted(decision.decision)} as not a rule`,
        title: ignored ? "It stops steering here" : "It stops steering here, and teammates who added it drop it once you commit the change",
      })));
}

function notRuleRow(decision) {
  return h("article.mrow.off", null,
    h("p.mq", null, decision.decision),
    h("div.mmeta", null, levelBadge(decision.level), scopeChips(decision.scope)),
    h("div.macts", null, button("", "marestore", decision.id, "Restore", { aria: `Restore ${quoted(decision.decision)}` })));
}

/* ---------- conflict cards ---------- */

/** A side's change as a diff: added lines green, removed red, headers muted. */
const hunkBlock = hunk => h("pre.mhunk", null, hunkLines(hunk).map(line => h(`span.${line.kind}`, null, line.text, "\n")));

/** Where a side's change is and when it was pushed. */
function sideWhere(side) {
  if (side.location === null && side.pushedAt === null) return null;
  return h("div.mloc", null,
    side.location === null ? null : h("code", null, side.location),
    side.pushedAt === null ? null : h("span", null, `pushed ${stamp(Date.parse(side.pushedAt))}`));
}

/** One side of a conflict: who (a link when its pull request is safe to open), what, the hunk and where. */
function sideBox(side, which) {
  const href = safeLink(side.url);
  const who = href === null ? sideHeading(side)
    : [h("a.mlink", { href, target: "_blank", rel: "noopener noreferrer", title: `Open ${side.label} on GitHub` }, side.label), side.author ? ` · ${side.author}` : null];
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

/** The four choices, all alike: the person decides, so none is recommended. */
function conflictButtons(conflict) {
  const about = quoted(conflict.title);
  return [
    button("", "mamine", conflict.id, "Keep mine", { aria: `Keep mine for ${about}` }),
    button("", "matheirs", conflict.id, "Take theirs", { aria: `Take theirs for ${about}` }),
    button("", "manew", conflict.id, "Write a new decision…", { aria: `Write a new decision for ${about}` }),
    button(".ghost", "manotconflict", conflict.id, "Not a conflict", { aria: `Mark ${about} as not a conflict` }),
  ];
}

/** What the dimmer measured: one plain sentence per signal, as the agent's card shows them. */
function conflictSize(reasons) {
  if (reasons.length === 0) return null;
  return h("div.msize", null, h("span.k", null, "Size"), h("ul.mreasons", null, reasons.map(line => h("li", null, line))));
}

/** Why a card is quieter than its level, or put off: queued over the budget, or snoozed with "later". */
function conflictState(conflict) {
  const lines = [conflict.queued ? QUEUED_TEXT : null, snoozeText(conflict)].filter(line => line !== null);
  return lines.length === 0 ? null : h("p.mstate", null, lines.join(". "));
}

function conflictCard(conflict) {
  const logged = conflict.level === "logged";
  const snooze = snoozeText(conflict);
  return h(`article.mcard.${conflict.level}`, { "data-kind": conflict.kind },
    h("div.mchead", null,
      h(`span.badge.mkind.${conflict.kind}`, null, KIND_TEXT[conflict.kind]),
      h(`span.badge.mlvl.${conflict.level}`, { title: LEVEL_TEXT[conflict.level] }, LEVEL_BADGE[conflict.level]),
      conflict.queued ? h("span.badge.mqueued", { title: QUEUED_TEXT }, "Queued") : null,
      snooze === null ? null : h("span.badge.msnoozed", { title: snooze }, "Later"),
      h("h3.mctitle", null, conflict.title),
      h("span.mshort", { title: `Name it in chat as #${conflict.shortId}` }, `#${conflict.shortId}`),
      h("span.mcage", null, "Open for ", ageSlot(), logged ? null : [" · ", LEVEL_TEXT[conflict.level]])),
    conflictState(conflict),
    h("div.mvs", null, sideBox(conflict.mine, "mine"), sideBox(conflict.theirs, "theirs")),
    conflictSize(conflict.reasons),
    whyConflict(conflict.evidence),
    h("div.meditslot"),
    h("div.macts", null, conflictButtons(conflict),
      logged ? h("span.mlogged", null, icon("logged"), LEVEL_TEXT.logged) : null));
}

const EDIT_LABEL = { mine: "Keep mine: the decision to save", theirs: "Take theirs: the decision to save", new: "Your new decision" };

/** The decision text a choice opens with; `patchDraft` keeps its counter and Confirm current. */
function draftEditor(conflict, draft) {
  const area = h("textarea.mdraft", { id: "ma-draft", rows: 3, "aria-describedby": "ma-draft-hint" });
  area.value = draft.text;
  return h("div.medit", null,
    h("label.k", { for: "ma-draft" }, EDIT_LABEL[draft.action]),
    area,
    h("div.meditfoot", null,
      h("span.mhint", { id: "ma-draft-hint" }),
      h("span.meditacts", null,
        h("button.btn.sm.ghost", { "data-act": "macancel", "data-i": conflict.id }, "Cancel"),
        h("button.btn.sm.solid", { "data-act": "maconfirm", "data-i": conflict.id }, "Confirm"))),
    h("p.mnote2", null, "Saved as a decision on this branch in ", h("code", null, ".rerail/candidates/"), " with a ledger entry in ", h("code", null, ".rerail/decisions/"), ". Commit both with your work."));
}

function draftHint(text) {
  if (text.trim() === "") return `1 to ${fmt(DECISION_MAX)} characters.`;
  return draftProblem(text) ?? `${fmt(text.trim().length)} of ${fmt(DECISION_MAX)} characters.`;
}

/** A card's draft: built when it opens, then patched (counter, Confirm, busy) so typing never rebuilds it. */
function patchDraft(node, conflict) {
  const draft = ui.mergeDraft?.id === conflict.id ? ui.mergeDraft : null;
  for (const [action, act] of Object.entries(OPENER)) {
    setAttr(node.querySelector(`button[data-act="${act}"]`), "aria-expanded", draft?.action === action ? "true" : "false");
  }
  const slot = node.querySelector(".meditslot");
  setContent(slot, draft === null ? "" : draft.action, () => (draft === null ? null : draftEditor(conflict, draft)));
  if (draft === null) return;
  const area = slot.querySelector("textarea");
  const problem = draftProblem(draft.text), invalid = problem !== null && draft.text.trim() !== "";
  setText(slot.querySelector(".mhint"), draftHint(draft.text));
  setClass(slot.querySelector(".mhint"), "bad", invalid);
  setAttr(area, "aria-invalid", invalid ? "true" : null);
  setAttr(area, "readonly", conflict.busy);
  setAttr(slot.querySelector('button[data-act="maconfirm"]'), "disabled", problem !== null || conflict.busy || ui.mergeSwitching);
}

/** What a card shows, for its key: a change rebuilds the card. Its age, busy state and draft are patched. */
const conflictContent = c => [c.status, c.level, c.queued, c.snooze, c.reasons, c.shortId, c.kind, c.title, c.mine, c.theirs, c.evidence];

function syncCards(list, items) {
  syncChildren(list, items, c => `${c.id}\u0000${JSON.stringify(conflictContent(c))}`, conflictCard, (node, conflict) => {
    setAttr(node, "aria-busy", conflict.busy ? "true" : null);
    for (const b of node.querySelectorAll("button[data-act]")) setAttr(b, "disabled", conflict.busy || ui.mergeSwitching);
    setText(node.querySelector(".mago"), ago(Date.parse(conflict.detectedAt)).replace(/ ago$/u, ""));
    patchDraft(node, conflict);
  });
}

/** A resolved or dismissed conflict, folded: how it ended, who decided (a person or precedent), and the rule path. */
function closedRow(conflict) {
  const r = resolutionText(conflict);
  const tone = r.badge === "Not a conflict" ? "" : r.precedent ? ".mprec" : ".mres";
  return h("article.mrow.mclosed", null,
    h("div.mchead", null, h(`span.badge${tone}`, null, r.badge), h("h3.mctitle", null, conflict.title)),
    r.text === null ? null : h("p.mq", null, r.text),
    h("div.mmeta", null,
      r.how === null ? null : h("span", null, r.how),
      r.who === null ? null : h("b.mauthor", null, r.who),
      r.at === null ? null : ageSlot(),
      h("span", null, `with ${sideHeading(conflict.theirs)}`),
      r.rule === null ? null : h("code.mrule", { title: "The decision saved on this branch" }, r.rule)));
}

/* ---------- the decision log ---------- */

const LOG_COLUMNS = ["When", "Status", "Outcome", "By", "Branch", "Rule"];

function logRow(entry) {
  const at = Date.parse(entry.recordedAt);
  return h("tr", null,
    h("td", { "data-label": "When" }, h("time", { datetime: entry.recordedAt, title: Number.isNaN(at) ? null : stamp(at) }, logWhen(entry.recordedAt))),
    h("td", { "data-label": "Status" }, h(`span.badge.mst.${entry.status}`, { title: LEDGER_STATUS_TITLE[entry.status] }, LEDGER_STATUS_TEXT[entry.status])),
    h("td", { "data-label": "Outcome" }, entry.outcome, h("span.sub", null, entry.text),
      entry.follows === null ? null : h("span.sub", null, "Follows ", h("code", null, entry.follows.candidateId), ` on ${entry.follows.ref}`)),
    h("td", { "data-label": "By" }, h(`span.badge.mby.${entry.by.kind}`, null, entry.by.name)),
    h("td", { "data-label": "Branch" }, entry.source.label),
    h("td", { "data-label": "Rule" }, entry.rule === null ? h("span.sub", { "aria-label": "None" }, "—") : h("code.mrule", null, entry.rule)));
}

/** One question: its title, then a table of its entries, newest first. */
function logQuestion(question) {
  return h("div.list.mlogwrap", null,
    h("h3.mlq", null, question.title),
    h("table.mlog", null,
      h("caption.vh", null, `Decision log for ${question.title}, newest first`),
      h("colgroup", null, LOG_COLUMNS.map(name => h(`col.c-${name.toLowerCase()}`))),
      h("thead", null, h("tr", null, LOG_COLUMNS.map(name => h("th", { scope: "col" }, name)))),
      h("tbody", null, question.items.map(logRow))));
}

/** The status filter: All, then each status the ledger holds, with its count. */
function logFilter(counts, chosen) {
  const button = (name, label, count) => h("button.mlogf", { "data-act": "malog", "data-i": name, "aria-pressed": (chosen ?? "all") === name ? "true" : "false" }, label, h("span.mtabn", null, fmt(count)));
  const total = Object.values(counts).reduce((sum, count) => sum + count, 0);
  return h("div.mlogfilter", { role: "group", "aria-label": "Show decisions by status" },
    button("all", "All", total),
    Object.entries(LEDGER_STATUS_TEXT).filter(([name]) => counts[name] > 0 || chosen === name).map(([name, label]) => button(name, label, counts[name])));
}

/**
 * Reconcile one list's rows: a row is rebuilt only when what it shows changes (its key holds that), and patched for
 * its write in flight and its age (`when`, an ISO time), so neither a poll nor a click moves focus off a button whose
 * row did not change.
 */
function syncRows(list, items, row, content, when = null) {
  syncChildren(list, items, item => `${item.id}\u0000${JSON.stringify(content(item))}`, row, (node, item) => {
    setAttr(node, "aria-busy", item.busy ? "true" : null);
    for (const b of node.querySelectorAll("button[data-act]")) setAttr(b, "disabled", item.busy || ui.mergeSwitching);
    const age = node.querySelector(".mago");
    if (age !== null && when !== null) setText(age, ago(Date.parse(when(item))));
  });
}

/* ---------- the page ---------- */

function slot(name) {
  const head = h("div.ghead"), list = h("div.list.mlist"), empty = h("div");
  return { root: h("section.msec", { "data-section": name }, head, list, empty), head, list, empty };
}

/** A folded list whose summary opens it (`mafold` with `name`), kept open across renders in `ui.mergeOpen`. */
function foldedList(name) {
  const list = h("div.list.mlist"), summary = h("summary", { "data-act": "mafold", "data-i": name });
  return { root: h("details.mfold", null, summary, h("div.in", null, list)), summary, list };
}

const TAB_LABEL = { conflicts: "Conflicts", decisions: "Decisions", log: "Decision log" };

/** The tabs: toggle buttons over the three panels, patched in place (`renderTabs`). */
function buildTabs() {
  const buttons = {}, panels = {};
  for (const name of MERGE_TABS) {
    buttons[name] = h("button.mtab", { "data-act": "matab", "data-i": name, "aria-controls": `ma-panel-${name}` }, TAB_LABEL[name]);
    panels[name] = h("div.mpanel", { id: `ma-panel-${name}`, role: "region", "aria-label": TAB_LABEL[name] });
  }
  const conflictCount = h("span.mtabn");
  buttons.conflicts.append(conflictCount);
  const tabs = h("div.mtabs", { role: "group", "aria-label": "Merge Assist views" }, MERGE_TABS.map(name => buttons[name]));
  return { tabs, buttons, panels, conflictCount };
}

function buildConflicts(panel) {
  const head = h("div.ghead"), list = h("div.mcards"), empty = h("div"), closed = foldedList("closedConflicts");
  panel.append(h("section.msec.mconf", { "data-section": "conflicts" }, head, list, empty, closed.root));
  return { head, list, empty, closed };
}

function buildLog(panel) {
  const head = h("div.ghead"), problems = h("div"), filter = h("div"), table = h("div"), empty = h("div");
  panel.append(h("section.msec.mlogsec", { "data-section": "log" }, head, problems, filter, table, empty));
  return { head, problems, filter, table, empty };
}

function build() {
  const head = h("div.ghead.mhead", null,
    h("h2", null, "Merge Assist", h("span.badge.mexp", null, "Experimental")),
    h("p.pitch", null, "Decisions you make while steering your coding agent travel with your branch. Rerail compares them with other branches and open pull requests before they merge."));
  const notice = h("div"), off = h("div"), strip = h("div");
  const tabs = buildTabs();
  const talk = slot("talk"), review = slot("review"), added = slot("added"), own = slot("own");
  const notRules = foldedList("notRules");
  own.root.append(notRules.root);
  tabs.panels.decisions.append(talk.root, review.root, added.root, own.root);
  page = {
    root: h("div.mpage", null, head, notice, off, strip, tabs.tabs, MERGE_TABS.map(name => tabs.panels[name])),
    notice, off, strip, tabs, talk, review, added, own,
    notRules: notRules.root, notRulesSummary: notRules.summary, notRulesList: notRules.list,
    conflicts: buildConflicts(tabs.panels.conflicts), log: buildLog(tabs.panels.log),
    // Built with the consent card and the strip; patched in place on every render.
    onButton: null, stripWhen: null, scanButton: null, offButton: null,
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
        h("li", null, "Rerail reads the corrections you give your coding agent in this repository and asks ", h("b", null, "your coding agent’s CLI"),
          " to summarise each one as a decision. That sends the correction and the code it corrected to your agent’s provider, as your session already does."),
        h("li", null, "Each decision is written to ", h("code", null, ".rerail/candidates/"), " in this checkout. Commit it to share it; Rerail never commits or pushes."),
        h("li", null, "Rerail reads your other local branches, and open pull requests through ", h("code", null, "gh"), " with your sign-in, read-only."),
        h("li", null, "Decisions from teammates with write access may steer your coding agent. Each one is listed here, and you can undo it.")),
      h("h3", null, "What Rerail found"),
      h("ul.mchecks", null, checks.map(check => h(`li.${check.ok ? "mready" : "mnotready"}`, { "data-check": check.key },
        icon(check.ok ? "done" : "logged", 14),
        h("span", null, h("span.vh", null, check.ok ? "Ready: " : "Note: "), segments(check.text),
          check.overview ? [" ", h("button.btn.sm.ghost.link", { "data-act": "nav", "data-i": "overview" }, "Open Overview")] : null)))),
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

/** The status strip: branch → base, judgment, gh, last scan (problems unfold), budget, then Scan now and Turn off. */
function renderStrip(status) {
  const show = status !== null && status.enabled;
  if (!show) {
    setContent(page.strip, "off", () => null);
    Object.assign(page, { stripWhen: null, scanButton: null, offButton: null });
    return;
  }
  const { branch, base } = branchText(status);
  const judge = judgeText(status.readiness.judge), github = githubText(status.readiness.github), budget = budgetText(status.budget);
  const scan = status.lastScan, problemsOpen = ui.mergeOpen.has("problems");
  const sig = JSON.stringify([branch, base, judge, github, budget, scan, problemsOpen, status.readiness.rulesActive]);
  setContent(page.strip, sig, () => {
    page.stripWhen = scan === null ? null : h("span.when");
    page.scanButton = h("button.btn.sm.solid.mscan", { "data-act": "mascan" });
    page.offButton = h("button.btn.sm.danger.quiet", { "data-act": "maoff", title: "Turn Merge Assist off for this repository" });
    return [
      status.readiness.rulesActive ? null : h("div.welcome.warn.mnote", null, icon("steered", 14),
        h("div", null, h("b", null, "Rerail isn’t set up and started in this checkout, so decisions can’t steer yet."),
          h("p.acts", null, h("button.btn.sm", { "data-act": "nav", "data-i": "overview" }, "Open Overview")))),
      h("div.list.mstrip", null,
        h("div.mcells", null,
          cell("Status", h("span.mon-state", null, h("span.dot", { "aria-hidden": "true" }), "On")),
          cell("Branch", [h("span.mono", null, branch), h("span.marrow", { "aria-hidden": "true" }, " → "), h("span.vh", null, " into "), h("span.mono", null, base)]),
          cell("Judgment", judge.ok ? judge.text : [h("span.mwarn-t", null, judge.text), judge.detail ? h("span.sub", null, judge.detail) : null]),
          cell("GitHub", github.ok ? github.text : h("span.mwarn-t", null, github.text)),
          cell("Last scan", scan === null ? "Not scanned yet" : [page.stripWhen, h("span.sub", null, scannedWhat(scan))]),
          cell("Judgment calls", h(budget.spent ? "span.mwarn-t" : "span", { title: budget.spent ? "The hourly limit is reached: judgment waits, exact checks still run." : null }, budget.text))),
        h("div.mstripacts", null, page.scanButton, page.offButton),
        scan === null || scan.problems.length === 0 ? null : h(`details.mfold.mproblems`, { open: problemsOpen },
          h("summary", { "data-act": "mafold", "data-i": "problems" }, `${fmt(scan.problems.length)} ${scan.problems.length === 1 ? "thing" : "things"} the last scan could not read`),
          h("div.in", null, h("ul.mbul", null, scan.problems.map(problem => h("li", null, problem)))))),
    ];
  });
  const scanning = ui.mergeScanning || status.scanning, switching = ui.mergeSwitching;
  setAttr(page.scanButton, "disabled", scanning || switching);
  setAttr(page.scanButton, "aria-busy", scanning ? "true" : null);
  setText(page.scanButton, scanning ? "Scanning…" : "Scan now");
  setAttr(page.offButton, "disabled", switching);
  setText(page.offButton, switching ? "Turning off…" : "Turn off");
  if (page.stripWhen !== null) setText(page.stripWhen, ago(Date.parse(scan.at)));
}

/** A section's heading with its count, and what it holds. */
function renderHead(s, title, count, lead) {
  setContent(s.head, JSON.stringify([title, count, lead]), () => [
    h("h2", null, title, h("span.n2", { "aria-label": `${fmt(count)} in this section` }, fmt(count))),
    h("p", null, segments(lead)),
  ]);
}

const emptyBlock = empty => h("div.list", null, h("div.empty", null, h("h3", null, empty.title), empty.lines.map(line => h("p", null, segments(line)))));

function renderSection(s, { show, title, items, lead, row, content, when = null, empty = null }) {
  setAttr(s.root, "hidden", !show);
  if (!show) return;
  renderHead(s, title, items.length, lead);
  setAttr(s.list, "hidden", items.length === 0);
  syncRows(s.list, items, row, content, when);
  setContent(s.empty, JSON.stringify(items.length === 0 ? empty : null), () => (items.length > 0 || empty === null ? null : emptyBlock(empty)));
}

/** What a finding row shows, for its key: a change rebuilds the row. Its age is patched in place (`syncRows`). */
const findingContent = f => [f.status, f.tier, f.candidate, f.peer, f.reason, f.evidence, f.message];

function peerEmpty(status, waiting) {
  const scan = status.lastScan;
  if (scan === null) {
    return { title: "No other branches scanned yet", lines: [["Rerail scans when a session starts here, after it saves a decision, and when you choose Scan now. It reads your other local branches and open pull requests."]] };
  }
  const read = `Rerail scanned ${scannedWhat(scan)} ${ago(Date.parse(scan.at))}`;
  return waiting
    ? { title: "Nothing added yet", lines: [[`${read}. The decisions it found wait above for you.`]] }
    : { title: "Nothing from other branches", lines: [[`${read} and found no decisions to add. Decisions appear here once teammates commit them to their branches.`]] };
}

function ownEmpty(status) {
  const lines = [["Correct your coding agent in a session here, e.g. “don’t wrap API calls in retry loops”. Rerail saves the decision and it applies to the rest of this branch’s work."]];
  if (!status.readiness.judge.available) lines.push(["Saving decisions needs Claude Code or Codex on your PATH."]);
  return { title: "No decisions on this branch yet", lines };
}

function renderSections(status) {
  const on = status !== null && status.enabled;
  const { talk, review, added, own, notRules } = on ? sectionsOf(status, ui.mergePending) : { talk: [], review: [], added: [], own: [], notRules: [] };
  renderSection(page.talk, {
    show: on && talk.length > 0, title: "Decided differently", items: talk, row: talkRow, content: findingContent,
    lead: ["Your coding agent’s CLI found these contradict a decision on this branch. Nothing is applied: talk it over with the author."],
  });
  renderSection(page.review, {
    show: on && review.length > 0, title: "Needs your review", items: review, row: reviewRow, content: findingContent,
    lead: ["Adopt applies a decision in this checkout; Dismiss hides it until it changes."],
  });
  renderSection(page.added, {
    show: on, title: "Added from other branches", items: added, row: addedRow, content: findingContent, when: f => f.updatedAt,
    lead: ["Decisions from other branches that steer your coding agent in this checkout. Undo removes one here only."],
    empty: on ? peerEmpty(status, talk.length + review.length > 0) : null,
  });
  const ignored = on && status.readiness.candidatesIgnored;
  renderSection(page.own, {
    show: on, title: "Decisions on this branch", items: own, row: d => ownRow(d, ignored),
    content: d => [d.decision, d.level, d.generality, d.scope, d.committed, d.amends, d.amendedBy, ignored], when: d => d.createdAt,
    lead: ignored
      ? ["Saved from your corrections. Git ignores ", { code: ".rerail/candidates/" }, " here, so they stay on this machine."]
      : ["Saved from your corrections in ", { code: ".rerail/candidates/" }, ". Commit them to share them; Rerail never commits."],
    empty: on ? ownEmpty(status) : null,
  });
  setAttr(page.notRules, "hidden", !on || notRules.length === 0);
  setAttr(page.notRules, "open", ui.mergeOpen.has("notRules"));
  setText(page.notRulesSummary, `Not rules (${fmt(notRules.length)})`);
  syncRows(page.notRulesList, notRules, notRuleRow, d => [d.decision, d.level, d.scope]);
}

/** The tabs while on: the one shown pressed and its panel alone visible; Conflicts carries its open count. */
function renderTabs(status) {
  const on = status !== null && status.enabled;
  const { tabs, buttons, panels, conflictCount } = page.tabs;
  setAttr(tabs, "hidden", !on);
  const shown = on ? tabOf(status, ui.mergeTab, ui.mergePending) : null;
  for (const name of MERGE_TABS) {
    setAttr(buttons[name], "aria-pressed", shown === name ? "true" : "false");
    setAttr(buttons[name], "disabled", ui.mergeSwitching);
    setAttr(panels[name], "hidden", shown !== name);
  }
  const open = on ? conflictSections(status, ui.mergePending).open.length : 0;
  setText(conflictCount, open === 0 ? "" : ` · ${fmt(open)}`);
}

function conflictsEmpty(status, closed) {
  const scan = status.lastScan;
  const lines = [scan === null
    ? ["Rerail looks for conflicts on every scan and after your coding agent’s edits: work on other branches and open pull requests that overlaps yours, and decisions that contradict this branch’s."]
    : [`Rerail compared this branch with ${scannedWhat(scan)} ${ago(Date.parse(scan.at))} and found nothing waiting for you.`]];
  lines.push(["When work overlaps, it shows here and you decide: keep yours, take theirs, or write a new decision."]);
  if (closed > 0) lines.push(["Earlier ones are under Resolved and dismissed."]);
  return { title: "No open conflicts", lines };
}

const closedContent = c => [c.status, c.title, c.theirs.label, c.theirs.author, c.resolution];

function renderConflicts(status) {
  const on = status !== null && status.enabled;
  const { open, closed } = on ? conflictSections(status, ui.mergePending) : { open: [], closed: [] };
  const c = page.conflicts;
  renderHead(c, "Open conflicts", open.length,
    ["Work in flight that overlaps yours. Rerail never picks a side: what you choose is saved as a decision on this branch, with an entry for the decision log, for you to commit."]);
  setAttr(c.list, "hidden", open.length === 0);
  syncCards(c.list, open);
  const empty = on && open.length === 0 ? conflictsEmpty(status, closed.length) : null;
  setContent(c.empty, JSON.stringify(empty), () => (empty === null ? null : emptyBlock(empty)));
  setAttr(c.closed.root, "hidden", closed.length === 0);
  setAttr(c.closed.root, "open", ui.mergeOpen.has("closedConflicts"));
  setText(c.closed.summary, `Resolved and dismissed (${fmt(closed.length)})`);
  syncRows(c.closed.list, closed, closedRow, closedContent, x => x.resolution?.at ?? x.detectedAt);
}

const LOG_EMPTY = {
  title: "No decisions recorded yet",
  lines: [["When you resolve a conflict, or an earlier verdict answers one, Rerail writes an entry to ", { code: ".rerail/decisions/" }, " on this branch. Commit it with your work: every checkout reads it from there, and it is final once it merges."]],
};

function renderLogProblems(problems) {
  setContent(page.log.problems, JSON.stringify(problems), () => (problems.length === 0 ? null
    : h("div.welcome.warn.mnote", { role: "status" }, icon("steered", 14),
      h("div", null, h("b", null, "Some decision entries could not be read."), h("ul.mbul", null, problems.map(p => h("li", null, p)))))));
}

function renderLog(status) {
  const on = status !== null && status.enabled;
  const counts = on ? logStatusCounts(status) : {};
  const total = Object.values(counts).reduce((sum, count) => sum + count, 0);
  const chosen = ui.mergeLogStatus;
  const questions = on ? logQuestions(status, chosen) : [];
  const l = page.log;
  renderHead(l, "Decision log", total,
    ["Every verdict on a conflict, by question: entries in ", { code: ".rerail/decisions/" }, " on the base branch, this branch and the branches in flight. In force once merged; provisional until then."]);
  renderLogProblems(on ? status.decisionLog.problems : []);
  setContent(l.filter, JSON.stringify([counts, chosen]), () => (total === 0 ? null : logFilter(counts, chosen)));
  setContent(l.table, JSON.stringify([questions, startOfDay()]), () => (questions.length === 0 ? null : h("div", null, questions.map(logQuestion))));
  setContent(l.empty, JSON.stringify(on && total === 0), () => (on && total === 0 ? emptyBlock(LOG_EMPTY) : null));
}

/** After a draft opens, its text; after one closes, the button that opened it. */
function applyFocus() {
  const target = focusNext;
  focusNext = null;
  if (target === null) return;
  const el = target.draft
    ? page.conflicts.list.querySelector("textarea.mdraft")
    : [...page.conflicts.list.querySelectorAll(`button[data-act="${target.act}"]`)].find(b => b.dataset.i === target.id);
  el?.focus();
}

/** The conflict a link names (`#merge?conflict=<id>`, the session's line), until a status shows whether it is here. */
let reveal = null;

export function enter(route) {
  if (route.conflict === undefined) return;
  reveal = route.conflict;
  ui.mergeTab = "conflicts";
}

/** Which conflict to bring into view this render, once the status is in; a closed one opens its fold. */
function revealTarget(status) {
  if (reveal === null || status === null) return null;
  const found = status.enabled ? status.conflicts.find(c => c.id === reveal) : undefined;
  reveal = null;
  if (found === undefined) return null;
  if (found.status !== "open") ui.mergeOpen.add("closedConflicts");
  return found;
}

/** Scroll the linked conflict's card (or folded row) into view and focus it, so a screen reader starts there. */
function showConflict(conflict) {
  if (conflict === null) return;
  const list = conflict.status === "open" ? page.conflicts.list : page.conflicts.closed.list;
  const node = [...list.children].find(n => n.dataset.key?.startsWith(`${conflict.id}\u0000`));
  if (node === undefined) return;
  setAttr(node, "tabindex", "-1");
  node.scrollIntoView?.({ block: "start", behavior: "smooth" });
  node.focus({ preventScroll: true });
}

export function render(root) {
  if (page === null || page.root.parentNode !== root) {
    build();
    root.replaceChildren(page.root);
  }
  // After a failed read the page says why and offers nothing to act on: the last status may no longer hold.
  const status = data.mergeAssistError === null ? data.mergeAssist : null;
  const linked = revealTarget(status);
  renderNotice(status);
  renderOff(status);
  renderStrip(status);
  renderTabs(status);
  renderConflicts(status);
  renderSections(status);
  renderLog(status);
  applyFocus();
  showConflict(linked);
}
