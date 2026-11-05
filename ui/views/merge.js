/**
 * Merge Assist (`#merge`) for the scoped checkout. Off: what turning it on does and what Rerail found (the agent CLI,
 * `gh`, whether Git ignores `.rerail/candidates/`, whether rules can steer here), then Turn on Merge Assist. On: the
 * status strip (branch and base, the judging CLI, `gh`, the last scan and its problems, the hourly judgment budget,
 * Scan now and Turn off), then Decided differently, Needs your review, Added from other branches and Decisions on
 * this branch, with the ones marked as not a rule folded under it. The sections come from `merge-model.js`. When the
 * status cannot be read, the page says why and shows nothing to act on until a read succeeds again.
 *
 * Decisions, authors, branch names, pull request titles and links are other people's text: every one is a text node
 * (`h()` children), never markup, and a pull request link is an `href` only when `safePeerUrl` accepts it.
 *
 * Buttons the page keeps busy (Turn on, Scan now, Turn off, a row's actions) are patched in place, never rebuilt for a
 * write in flight or the passing of time, so the focus stays on the button a person used.
 */
import { h, icon, setAttr, setContent, setText, syncChildren } from "../dom.js";
import { ago, failed, fmt } from "../format.js";
import { data, ui, repoName } from "../state.js";
import { captureScope, inScope, writeMergeAssist } from "../poll.js";
import { copyText, invalidate, toast } from "../shell.js";
import {
  authorOf, branchText, budgetText, githubText, judgeText, levelText, levelTitle, peerLabel, quoted, readinessChecks,
  safePeerUrl, scanSummary, scannedWhat, sectionsOf, writeError,
} from "../merge-model.js";

export const needs = ["mergeAssist"];

const SETTINGS = "/api/merge-assist/settings";
const SCAN = "/api/merge-assist/scan";
const FINDINGS = "/api/merge-assist/findings";
const CANDIDATES = "/api/merge-assist/candidates";

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
};

/* ---------- pieces ---------- */

/** Text segments from `merge-model.js`: a string, or `{ code }` for a command or path. */
const segments = parts => parts.map(part => (typeof part === "string" ? part : h("code", null, part.code)));

const scopeChips = scope => h("span.mscope", { "aria-label": "Folders" },
  scope.length === 0 ? h("span.badge", null, "Whole repository") : scope.map(glob => h("span.badge.mono", null, glob)));

const levelBadge = level => h(`span.badge.mlevel.${level}`, { title: levelTitle(level) }, levelText(level));

/** A time a row shows ("4m ago"); `syncRows` fills it on every render, so it moves on without rebuilding the row. */
const ageSlot = () => h("span.mago");

/**
 * Where a finding came from: the pull request (a link when it is safe to open) and its title, or the branch; then who
 * is behind it: the pull request's author as GitHub reports it, or the author a branch's decision file declares.
 */
function source(finding) {
  const { peer } = finding;
  const status = data.mergeAssist;
  const href = peer.kind === "pull-request" && status !== null
    ? safePeerUrl(peer.url, { repositoryKey: status.repositoryKey, repository: status.readiness.github.repository }) : null;
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

function build() {
  const head = h("div.ghead.mhead", null,
    h("h2", null, "Merge Assist", h("span.badge.mexp", null, "Experimental")),
    h("p.pitch", null, "Decisions you make while steering your coding agent travel with your branch. Rerail compares them with other branches and open pull requests before they merge."));
  const notice = h("div"), off = h("div"), strip = h("div");
  const talk = slot("talk"), review = slot("review"), added = slot("added"), own = slot("own");
  const notRules = h("details.mfold"), notRulesSummary = h("summary", { "data-act": "mafold", "data-i": "notRules" }), notRulesList = h("div.list.mlist");
  notRules.append(notRulesSummary, h("div.in", null, notRulesList));
  own.root.append(notRules);
  page = {
    root: h("div.mpage", null, head, notice, off, strip, talk.root, review.root, added.root, own.root),
    notice, off, strip, talk, review, added, own, notRules, notRulesSummary, notRulesList,
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

function renderSection(s, { show, title, items, lead, row, content, when = null, empty = null }) {
  setAttr(s.root, "hidden", !show);
  if (!show) return;
  renderHead(s, title, items.length, lead);
  setAttr(s.list, "hidden", items.length === 0);
  syncRows(s.list, items, row, content, when);
  setContent(s.empty, JSON.stringify(items.length === 0 ? empty : null), () => (items.length > 0 || empty === null ? null
    : h("div.list", null, h("div.empty", null, h("h3", null, empty.title), empty.lines.map(line => h("p", null, segments(line)))))));
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

export function render(root) {
  if (page === null || page.root.parentNode !== root) {
    build();
    root.replaceChildren(page.root);
  }
  // After a failed read the page says why and offers nothing to act on: the last status may no longer hold.
  const status = data.mergeAssistError === null ? data.mergeAssist : null;
  renderNotice(status);
  renderOff(status);
  renderStrip(status);
  renderSections(status);
}
