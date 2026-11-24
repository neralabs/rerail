/**
 * The sheet. A rule, as the v8 design reads it: the switch and "Edit with coding agent" in the header, then what
 * Rerail does on a violation (read-only here), its numbers (logged and declined in the range, logged since install),
 * its description, where it came from and when, where it
 * applies, what the coding agent sees (Edit hands a new text to the coding agent), its recent violations, and for a
 * rule created by you a Delete in a footer at the end. A rule pack's contents render here too (`pack-panel.js`).
 */
import { h, setAttr, setText, setContent, icon, pendingIcon } from "../dom.js";
import { andList, fmt, plural, whenLabel, clock, violationOwner } from "../format.js";
import { ruleMode, modeLabel, isLog, isFromPack, packOriginName, UNKNOWN_PACK_ORIGIN, awaitingApproval, awaitingYourApproval, notRunningReason, scopeText, conditionText, messageText, MODE_PLAIN } from "../derive.js";
import { data, ui, rangeCaption, fires, allFires, isMeasured, ruleFindings, ruleFindingsTotal, titleOf, packOfRule, attachedPackOf, repoName } from "../state.js";
import { renderPackPanel } from "./pack-panel.js";
import { MODE_READ_ONLY, enforcementTag, updateEnforcementTag } from "./enforcement.js";
import { outcomesOf } from "../history-model.js";
import { ASK, ASK_OPEN, LOG_NOTE, askPrompt, agentSeesHint, askHint, approveLabel, fmtDay, canDelete, deleteConsequences, splitPath } from "./rule-sheet-model.js";
import { EDIT_TITLE, changedParts, editedLine, pendingEditOf } from "./rule-edits.js";
import { sheetState, syncSheetState, askRule } from "./rule-sheet-actions.js";

const kv = rows => rows.flatMap(([k, v, cls]) => [h("dt", null, k), h("dd", { class: cls }, v)]);
const RECENT_SHOWN = 3;

/** At most two sentences of a description; the pack copy keeps them short upstream too. */
function twoSentences(text) {
  const sentences = String(text).match(/[^.!?]+[.!?]+(\s|$)/gu);
  return sentences && sentences.length > 2 ? `${sentences.slice(0, 2).join("").trim()}…` : String(text).trim();
}

const capitalize = s => (s ? s[0].toUpperCase() + s.slice(1) : s);
const repoLabel = () => repoName() || "this repo";

/** What the coding agent is told on a violation: the newest violation's message, else the rule's own. */
const agentSees = (stored, id) => ruleFindings(id)[0]?.message ?? messageText(stored);

/* ---------- rule ---------- */

let p = null;

function buildRule() {
  const el = {};
  el.back = h("button.back", { "data-act": "panelback" }, "← Back to list");
  el.title = h("h2");
  el.sw = h("button.sw", { role: "switch", "aria-checked": "false", "data-act": "toggle" });
  el.swText = h("span");
  el.ask = h("button.btn.sm.solid", { "data-act": "asktoggle", "aria-expanded": "false" }, "Edit with coding agent");
  el.askChips = h("div.chips", null, ASK.map(([k, l]) => h("button.chip", { "data-act": "askintent", "data-i": k, "aria-pressed": "false" }, l)));
  el.askText = h("textarea", { id: "askText", readonly: true, rows: 3, "aria-label": "Prompt for your coding agent" });
  el.askWhere = h("p.why.top");
  el.askWhy = h("p.why");
  el.pask = h("div.pask.sask", { hidden: true },
    h("button.x", { "data-act": "asktoggle", "aria-label": "Close" }, "✕"),
    h("p.lbl", null, "Edit with coding agent"), el.askWhere, el.askChips,
    h("div.cmd", null, el.askText, h("button.btn.sm.solid", { "data-act": "askcopy" }, icon("copy"), "Copy")),
    el.askWhy);
  el.enforcement = enforcementTag();
  el.modeText = h("p");
  // Each card reads the same three lines, so their numbers and words line up: how many, what, over when.
  const stat = (what, tip) => h("div", { title: tip }, h("b.num"), h("span", null, what), h("small"));
  el.stats = h("div.pstats", null,
    stat("logged", "Violations of this rule Rerail logged in the range, steered or not."),
    stat("declined", "Violations whose steer the coding agent declined, recording why with rerail feedback."),
    stat("logged", "Violations of this rule Rerail logged since it was installed."));
  el.statsNone = h("p.pstats-none");
  el.description = h("p.desc");
  el.source = h("dl.kv");
  el.applies = h("p");
  el.sees = h("section.psec.sees");
  el.recent = h("div");
  el.foot = h("div.pfoot");
  el.approval = h("div.pnotice");
  el.notRunning = h("div.pnotice");
  el.body = h("div.pbody", null, el.pask, el.notRunning, el.approval,
    h("div.enf", null, h("div.enfhead", null, h("div.lbl", null, "On a violation"), el.enforcement), el.modeText, h("p.enfhint", null, MODE_READ_ONLY)),
    el.stats, el.statsNone,
    h("section.psec", null, h("h3", null, "Description"), el.description),
    h("section.psec", null, h("h3", null, "Source"), el.source),
    h("section.psec", null, h("h3", null, "Where it applies"), el.applies),
    el.sees,
    h("section.psec.sep", null, h("h3", null, "Recent history"), el.recent),
    el.foot);
  el.root = h("div.pwrap", null,
    h("div.phead", null,
      h("div.ptop", null, h("div", null, el.back, el.title), h("button.x", { "data-act": "close", "aria-label": "Close" }, "✕")),
      h("div.pact", null, el.sw, el.swText, h("span.grow"), el.ask)),
    el.body);
  p = el;
}

function renderAsk(stored) {
  setAttr(p.ask, "aria-expanded", String(ui.askOpen));
  // An open box marks its button instead of a second primary beside the box's own Copy.
  setAttr(p.ask, "class", ui.askOpen ? "btn sm open" : "btn sm solid");
  setAttr(p.pask, "hidden", !ui.askOpen);
  if (!ui.askOpen) return;
  for (const c of p.askChips.children) setAttr(c, "aria-pressed", String(c.dataset.i === ui.askIntent));
  const prompt = askPrompt(askRule(stored), ui.askIntent);
  if (p.askText.value !== prompt) p.askText.value = prompt;
  setText(p.askWhere, `Pick what to change, then paste the prompt into a coding agent session in ${repoLabel()}.`);
  setText(p.askWhy, `${ASK_OPEN[ui.askIntent] ? "Finish the sentence after pasting. " : ""}${ui.askIntent === "ask"
    ? "Your coding agent answers from the rule and its History."
    : askHint({ fromPack: isFromPack(stored) })}`);
}

/** What the rule does on a violation: the tag on the "On a violation" line, and the sentence that says what that means. */
function renderModes(stored) {
  updateEnforcementTag(p.enforcement, stored);
  // A rule that runs nowhere does nothing on a violation: the line says what it is set to, for when it runs again.
  setText(p.modeText, notRunningReason(stored) === null
    ? MODE_PLAIN[ruleMode(stored)] ?? ""
    : `Set to ${modeLabel(stored)}. Nothing is logged or steered while it does not run.`);
}

/** The three cards' count and caption: logged and declined in the range, logged since install; dashes if uncounted. */
function statCards(id) {
  if (!isMeasured(id)) return Array.from({ length: 3 }, () => ["–", "not counted yet"]);
  const when = rangeCaption();
  return [[fmt(fires(id)), when], [fmt(outcomesOf(id).declined), when], [fmt(allFires(id)), "since install"]];
}

function renderStats(stored, id) {
  // A measured rule that never logged one says so in a line; the three cells would all read nothing.
  const none = isMeasured(id) && allFires(id) === 0;
  setAttr(p.stats, "hidden", none);
  setAttr(p.statsNone, "hidden", !none);
  // Off is off in every folder: `enabled` also reads off for a rule on in only some of them.
  if (none) { setText(p.statsNone, stored.folderStates.every(state => !state.on) ? "Off. Nothing is logged or steered." : "No violations logged yet."); return; }
  statCards(id).forEach(([count, caption], i) => {
    const card = p.stats.children[i];
    setText(card.firstChild, count);
    setText(card.lastChild, caption);
  });
}

/** The Approve button, which approves exactly the content this page read; `label` says what approving does. */
const approveButton = (subject, label = "Approve") => h("p.acts", null,
  h("button.btn.sm.solid", { "data-act": "approve", "data-i": subject.id, "data-kind": subject.kind, "data-digest": subject.digest }, label));

/**
 * What approving a waiting rule does: approval lifts the hold on a rule set to steer, so it steers; a rule set to Log
 * stays on Log only.
 */
const onceApproved = stored => (stored.modeSource?.by === "awaiting-approval"
  ? "then approve it, and Rerail tells your coding agent as it works, so it fixes what this rule catches."
  : "then approve it. It’s set to Log only, so Rerail keeps recording what it catches without telling your coding agent.");

/**
 * A rule your coding agent added that waits: who added it, what it caught in Log, and Approve, here after it is read;
 * "Approve and steer" for one set to steer, which approving lets steer.
 */
const yourRuleNotice = (stored, subject, caught) => h("div.welcome.warn", { role: "status" }, pendingIcon(), h("div", null,
  h("b", null, "Added by your coding agent, on Log only."),
  h("p", null, caught === 0 ? "Nothing caught in Log yet. " : `Caught ${fmt(caught)} ${plural(caught, "violation")} in Log so far, and your coding agent wasn’t told. `,
    `Review what it checks and what it tells the coding agent, ${onceApproved(stored)}`),
  approveButton(subject, approveLabel(stored))));

/**
 * For an edited copy (of a rule Rerail ships, or of your approved rule a coding agent edited): which rule, and that
 * approving the copy turns it off here.
 */
function copyLine(stored) {
  const id = stored.copiedFromRuleId, original = data.rules.get(id);
  const what = original === undefined ? `“${id}”` : isFromPack(original) ? `“${titleOf(original)}”, a rule Rerail ships` : `your rule “${titleOf(original)}”`;
  return h("p", null, h("b", null, "This is an edited copy"), ` of ${what}. Approving it turns the original off in ${repoName() || "this repo"}.`);
}

/** A rule or pack someone else wrote (a worktree's own rule, a pack Rerail did not publish, an edited copy) that waits. */
const otherNotice = (stored, subject, pack) => h("div.welcome.warn", { role: "status" }, pendingIcon(), h("div", null,
  h("b", null, subject.kind === "pack" ? `Its pack, ${pack?.name ?? subject.id}, waits for approval.` : "This rule waits for approval."),
  h("p", null, subject.kind === "pack"
    ? "Rerail did not publish it, so every rule in it runs in Log, steering no one, until a person approves what this version ships."
    : "Rerail did not write it, so it runs in Log, steering no one, until a person approves what it says. An edit waits again."),
  stored.copiedFromRuleId ? copyLine(stored) : null,
  h("p", null, "Read what it tells the coding agent first, then approve it here. Your coding agent can audit it first: ", h("span.mono", null, "/rerail-create approve the rules"), "."),
  approveButton(subject)));

/**
 * For a rule switched on that runs nowhere (its options don't apply, or Rerail does not run its document), that it
 * does not run and why: its switch and mode below say what it would do; nothing for a rule that runs or is off.
 */
function renderNotRunning(stored) {
  const reason = notRunningReason(stored);
  setContent(p.notRunning, JSON.stringify([reason]), () => (reason === null ? null
    : h("div.welcome.warn", { role: "status" }, icon("steered", 22), h("div", null,
      h("b", null, "On, but not running."),
      h("p", null, reason)))));
}

/**
 * For a rule a coding agent edited, whose copy waits (`rule-edits.js`): that the edit waits for approval, when it was
 * made, which of the sheet's parts it changes, Review the edit, which opens the copy's sheet where it is approved, and
 * ✕, which dismisses the notice for this edit.
 */
function editNotice(stored, copy) {
  const parts = changedParts(stored, copy);
  return h("div.welcome.edited", { role: "status" }, icon("pencil", 16), h("div.editbody", null,
    h("b", null, EDIT_TITLE),
    h("p", null, editedLine(copy)),
    parts.length ? h("p", null, `What changed: ${andList(parts)}.`) : null,
    h("p.acts", null, h("button.btn.sm", { "data-act": "detail", "data-i": copy.rule.id }, "Review the edit"))),
  h("button.x", { "data-act": "dismissedit", "data-i": copy.approval.subject.digest, "aria-label": "Dismiss", title: "Dismiss" }, "✕"));
}

/** The notice of a rule that needs no approval itself: that an edit of it waits, while the notice is not dismissed. */
function renderEdit(stored) {
  const copy = pendingEditOf(stored);
  const key = copy === null ? null : [stored.rule.id, copy.rule, copy.approval.subject.digest, copy.addedAt, stored.rule];
  setContent(p.approval, JSON.stringify(["edit", key]), () => (copy === null ? null : editNotice(stored, copy)));
}

/**
 * While the rule's content (or its pack's) waits for a person's approval, what that means and an Approve button that
 * approves exactly the content this page read; once approved, or for a rule that needs none, whether an edit of it waits.
 */
function renderApproval(stored) {
  const waiting = awaitingApproval(stored), subject = stored.approval?.subject;
  if (!waiting) { renderEdit(stored); return; }
  const pack = subject.kind === "pack" ? data.packs.find(x => x.approval?.subject.id === subject.id) : undefined;
  const yours = awaitingYourApproval(stored), caught = yours ? allFires(stored.rule.id) : 0;
  const key = [subject, pack?.name, yours, caught, stored.modeSource?.by, stored.copiedFromRuleId];
  setContent(p.approval, JSON.stringify(["approval", key]), () => (yours ? yourRuleNotice(stored, subject, caught) : otherNotice(stored, subject, pack)));
}

/** Where the rule comes from (its pack, or created by you) and when it was added here and last edited. */
function renderSource(stored, id) {
  const pack = attachedPackOf(stored) ?? packOfRule(stored);
  // A third party's pack names its publisher beside it: Rerail has not reviewed it.
  const from = pack ? ["Rule pack", packOriginName(pack)]
    : ["Origin", isFromPack(stored) ? `${UNKNOWN_PACK_ORIGIN}, added on its own` : "Created by you"];
  const updated = stored.updatedAt ?? null;
  setContent(p.source, JSON.stringify([id, from, stored.addedAt, updated]), () => kv([
    from,
    ["Rule id", id, "mono"],
    ["Added", fmtDay(stored.addedAt)],
    ...(updated === null ? [] : [["Updated", fmtDay(updated)]]),
  ]));
}

/** What the coding agent sees, read-only, with Edit handing a new text to the coding agent. */
function renderSees(stored, id) {
  const message = agentSees(stored, id);
  setAttr(p.sees, "hidden", message === "");
  if (message === "") return;
  const editing = sheetState.agentEdit;
  const log = isLog(stored);
  // Keyed on the edit state, not the text, so a poll never rebuilds the draft while it is being written.
  setContent(p.sees, editing ? `edit|${id}` : `show|${id}|${message}|${log}`, () => [
    h("div.h3row", null, h("h3", null, "What the agent sees"),
      editing ? null : h("button.btn.sm.ghost.link", { "data-act": "promptedit" }, "Edit")),
    editing ? agentSeesEdit(stored, message) : [
      h("p", null, h("span.said2", null, message)),
      log ? h("p.muted.note2", null, LOG_NOTE) : null,
    ],
  ]);
}

/** The hand-off: an editable draft of the text and what saving it does; Copy prompt asks the coding agent to save it. */
function agentSeesEdit(stored, message) {
  const draft = h("textarea", { id: "agentDraft", rows: 3, "aria-label": "New text for what the agent sees" });
  draft.value = message;
  return h("div.pask.sees-edit", null,
    draft,
    h("p.why", null, agentSeesHint({ fromPack: isFromPack(stored), log: isLog(stored) })),
    h("div.foot", null, h("span"), h("span.btns",
      null,
      h("button.btn.sm", { "data-act": "promptcancel" }, "Cancel"),
      h("button.btn.sm.solid", { "data-act": "promptcopy" }, icon("copy"), "Copy prompt"))));
}

function ownerButton(owner) {
  return h("button.open", { "data-act": "session", "data-i": owner.session, title: owner.title, disabled: owner.session === null }, owner.label);
}

/** One violation: what the agent was told (when it differs from the rule's text), the file and its folder, when, and its session. */
function violationRow(x, said) {
  const { file, dir } = splitPath(x.path);
  return h("div.rrow", null,
    x.message && x.message !== said ? h("span.m", null, x.message) : null,
    file ? h("span.file.mono", { title: x.path }, file) : h("span.file.muted", null, "No path recorded"),
    ownerButton(violationOwner(x)),
    h("span.dir.mono", { title: dir }, dir),
    h("span.when", null, `${capitalize(whenLabel(x.at))} · ${clock(x.at)}`));
}

function renderRecent(stored, id) {
  const top = ruleFindings(id).slice(0, RECENT_SHOWN), all = allFires(id), total = ruleFindingsTotal(id);
  const said = agentSees(stored, id);
  setContent(p.recent, `${id}|${top.map(x => `${x.auditId}:${x.id}`).join(",")}|${total}|${all}|${ui.range}|${said}`, () => {
    // A measured rule that never logged one already says so above the description.
    if (top.length === 0) return h("p.muted", null, all ? "Nothing in this range."
      : isMeasured(id) ? "None yet." : "Not counted in this release.");
    return [h("div.ract", null, top.map(x => violationRow(x, said))),
      h("button.btn.sm.ghost.link", { "data-act": "actrule", "data-i": id, style: "margin-top:6px" },
        total > RECENT_SHOWN ? `See all ${fmt(total)} in History` : "See it in History")];
  });
}

/** Delete, for a rule created by you: a quiet button, then what it does and the second click. */
function renderFoot(stored, id) {
  const deletable = canDelete(stored);
  setAttr(p.foot, "hidden", !deletable);
  if (!deletable) return;
  const confirm = sheetState.delConfirm, past = allFires(id), repo = repoLabel();
  setContent(p.foot, JSON.stringify([id, confirm, past, stored.enabled, repo]), () => (confirm
    ? h("div.pask.pdelbox", null,
      h("p.lbl", null, `Delete “${id}”?`),
      h("ul", null, deleteConsequences({ repo, pastViolations: past, on: stored.enabled }).map(line => h("li", null, line))),
      h("div.foot", null, h("span"), h("span.btns", null,
        h("button.btn.sm", { "data-act": "delcancel" }, "Cancel"),
        h("button.btn.sm.danger.solid", { "data-act": "delgo", "data-i": id }, "Delete rule"))))
    : h("button.btn.sm.danger.quiet", { "data-act": "delask", "aria-expanded": "false" }, "Delete rule")));
}

function renderRule(root, stored) {
  if (p === null) buildRule();
  if (p.root.parentNode !== root) { root.replaceChildren(p.root); delete root.dataset.sig; }
  const id = stored.rule.id;
  syncSheetState(id);
  setText(p.back, ui.backToPack ? `← Back to ${data.packs.find(x => x.id === ui.backToPack)?.name ?? "the rule pack"}` : "← Back to list");
  setAttr(p.back, "class", ui.backToPack ? "back show" : "back");
  setText(p.title, titleOf(stored));
  setAttr(p.title, "title", titleOf(stored));
  setAttr(p.sw, "aria-checked", String(stored.enabled));
  setAttr(p.sw, "aria-label", `Turn ${titleOf(stored)} ${stored.enabled ? "off" : "on"}`);
  setAttr(p.sw, "data-i", id);
  setText(p.swText, stored.enabled ? "On" : "Off");
  renderAsk(stored);
  renderModes(stored);
  renderNotRunning(stored);
  renderApproval(stored);
  renderStats(stored, id);
  setText(p.description, twoSentences(conditionText(stored) || `${titleOf(stored)}.`));
  renderSource(stored, id);
  setText(p.applies, scopeText(stored));
  renderSees(stored, id);
  renderRecent(stored, id);
  renderFoot(stored, id);
}

/** Render whichever sheet `ui.panel` names, and mark the page so the grid makes room for it. */
export function renderPanel(root) {
  const panel = ui.panel;
  const stored = panel?.kind === "rule" ? data.rules.get(panel.id) : undefined;
  const pack = panel?.kind === "pack" ? data.packs.find(x => x.id === panel.id) : undefined;
  // A browser that lost the daemon's capability still holds what it read before: the page asking to connect it shows
  // alone. `ui.panel` stays, so the sheet opens again once the browser is connected.
  const open = !data.accessRequired && (stored !== undefined || pack !== undefined);
  const state = open ? "open" : "";
  if (document.body.dataset.panel !== state) document.body.dataset.panel = state;
  if (!open) return;
  if (stored) renderRule(root, stored);
  else renderPackPanel(root, pack);
}

/** Scroll the body back to the top when a different rule opens. */
export function resetPanelScroll() {
  if (p !== null) p.body.scrollTop = 0;
}
