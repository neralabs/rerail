/**
 * The sheet. A rule, as the v8 design reads it: the switch and "Edit with coding agent" in the header, then what
 * Rerail does on a violation (read-only here), its numbers, its description, where it came from and when, where it
 * applies, what the coding agent sees (Edit hands a new text to the coding agent), its recent violations, and for a
 * rule created by you a Delete in a footer at the end. A rule pack's contents render here too (`pack-panel.js`).
 */
import { h, setAttr, setText, setContent, icon } from "../dom.js";
import { fmt, whenLabel, clock, shortSessionId } from "../format.js";
import { ruleMode, isLog, isFromPack, isRerailPack, publisherLabel, scopeText, conditionText, messageText, MODE_PLAIN } from "../derive.js";
import { data, ui, rangeLabel, fires, allFires, isMeasured, activityAllOf, ruleFindings, ruleFindingsTotal, titleOf, packOfRule, attachedPackOf, repoName } from "../state.js";
import { renderPackPanel } from "./pack-panel.js";
import { ASK, ASK_OPEN, askPrompt, agentSeesPrompt, fmtDay, canDelete, deleteConsequences, splitPath } from "./rule-sheet-model.js";
import { sheetState, syncSheetState, askRule } from "./rule-sheet-actions.js";

const kv = rows => rows.flatMap(([k, v, cls]) => [h("dt", null, k), h("dd", { class: cls }, v)]);
const MODES = [["log", "Log", "logged"], ["steer", "Steer", "steered"], ["block", "Block", "blocked"]];
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
  el.modes = h("div.rangectl.modes", { role: "group", "aria-label": "What Rerail does on a violation" },
    MODES.map(([v, l, o]) => h("button", { "data-v": v, disabled: true, "aria-pressed": "false" }, icon(o, 14), l)));
  el.modeText = h("p");
  el.stats = h("div.pstats", null, h("div", null, h("b.num"), h("span")), h("div", null, h("b"), h("span", null, "last logged")), h("div", null, h("b.num"), h("span", null, "since install")));
  el.description = h("p.desc");
  el.source = h("dl.kv");
  el.applies = h("p");
  el.sees = h("section.psec.sees");
  el.recent = h("div");
  el.foot = h("div.pfoot");
  el.body = h("div.pbody", null, el.pask,
    h("div.enf", null, h("div.lbl", null, "On a violation"), el.modes, el.modeText),
    el.stats,
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
    : "Your coding agent changes the rule by its id. The change applies from the next audit."}`);
}

function renderModes(stored) {
  const mode = ruleMode(stored);
  for (const b of p.modes.children) setAttr(b, "aria-pressed", String(b.dataset.v === mode));
  setText(p.modeText, mode === "block" ? MODE_PLAIN.block : `${MODE_PLAIN[mode] ?? ""} Block is not in this version.`);
}

function renderStats(id) {
  const measured = isMeasured(id), f = fires(id), all = allFires(id), lastSeen = activityAllOf(id).lastSeenAt;
  const [s1, s2, s3] = p.stats.children;
  setText(s1.firstChild, measured ? fmt(f) : "–");
  setText(s1.lastChild, measured ? `logged ${rangeLabel()}` : "not counted yet");
  setText(s2.firstChild, all && lastSeen ? whenLabel(lastSeen) : "never");
  setText(s3.firstChild, measured ? fmt(all) : "–");
}

/** Where the rule comes from (its pack, or created by you) and when it was added here and last edited. */
function renderSource(stored, id) {
  const pack = attachedPackOf(stored) ?? packOfRule(stored);
  // A third party's pack names its publisher beside it: Rerail has not reviewed it.
  const from = pack ? ["Rule pack", isRerailPack(pack) ? pack.name : `${pack.name} · ${publisherLabel(pack)}`]
    : ["Origin", isFromPack(stored) ? "By Rerail, added on its own" : "Created by you"];
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
    editing ? agentSeesEdit(id, message) : [
      h("p", null, h("span.said2", null, message)),
      log ? h("p.muted.note2", null, "On Log, Rerail keeps this for you; the agent is told only once the rule is set to Steer or Block.") : null,
    ],
  ]);
}

/** The hand-off: an editable draft of the text and the prompt that asks the coding agent to make it the message. */
function agentSeesEdit(id, message) {
  const draft = h("textarea", { id: "agentDraft", rows: 3, "aria-label": "New text for what the agent sees" });
  draft.value = message;
  return h("div.pask.sees-edit", null,
    draft,
    h("p.why", null, "Copy prompt copies: ", h("span.mono", null, agentSeesPrompt(id, "<your text>")),
      ". Paste it into a coding agent session; the agent sees the new text from its next check."),
    h("div.foot", null, h("span"), h("span.btns",
      null,
      h("button.btn.sm", { "data-act": "promptcancel" }, "Cancel"),
      h("button.btn.sm.solid", { "data-act": "promptcopy" }, icon("copy"), "Copy prompt"))));
}

/** One violation: what the agent was told (when it differs from the rule's text), the file and its folder, when, and its session. */
function violationRow(x, said) {
  const { file, dir } = splitPath(x.path);
  return h("div.rrow", null,
    x.message && x.message !== said ? h("span.m", null, x.message) : null,
    file ? h("span.file.mono", { title: x.path }, file) : h("span.file.muted", null, "No path recorded"),
    h("button.open", { "data-act": "session", "data-i": x.sessionId, title: "Open this session" }, shortSessionId(x.sessionId, x.host)),
    h("span.dir.mono", { title: dir }, dir),
    h("span.when", null, `${capitalize(whenLabel(x.at))} · ${clock(x.at)}`));
}

function renderRecent(stored, id) {
  const top = ruleFindings(id).slice(0, RECENT_SHOWN), all = allFires(id), total = ruleFindingsTotal(id);
  const said = agentSees(stored, id);
  setContent(p.recent, `${id}|${top.map(x => `${x.auditId}:${x.id}`).join(",")}|${total}|${all}|${ui.range}|${said}`, () => {
    if (top.length === 0) return h("p.muted", null, all ? `None logged ${rangeLabel()}. Widen the range to see earlier ones.` : "No violations logged yet. Rerail is silent until this rule matches.");
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
      h("div.foot", null, h("span", null, "You can undo this for a few seconds."), h("span.btns", null,
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
  renderStats(id);
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
  const open = stored !== undefined || pack !== undefined;
  const state = open ? "open" : "";
  if (document.body.dataset.panel !== state) document.body.dataset.panel = state;
  if (stored) renderRule(root, stored);
  else if (pack) renderPackPanel(root, pack);
}

/** Scroll the body back to the top when a different rule opens. */
export function resetPanelScroll() {
  if (p !== null) p.body.scrollTop = 0;
}
