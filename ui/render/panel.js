/**
 * The side panel. A rule (W1b, W3): what it checks with a caught and a passing example, what Rerail
 * does on a violation (the rule's mode, Log / Steer / Block, read-only here), its numbers, where it came
 * from (with the folders it is added at and what decided its on and mode) and its recent activity.
 * Create a rule (W2) and a rule pack's contents render here too.
 */
import { h, setAttr, setText, setContent, icon } from "../dom.js";
import { fmt, plural, whenLabel, clock, shortSessionId } from "../format.js";
import { kindLabel, modeLabel, ruleMode, isLog, isByRerail, originLabel, scopeText, technical, conditionText, messageText, folderText, deciderText, MODE_LABEL, MODE_PLAIN } from "../derive.js";
import { data, ui, rangeLabel, fires, allFires, isMeasured, activityAllOf, ruleFindings, titleOf, originChip, packRuleOf, packOfRule, attachedPackOf, repoName } from "../state.js";
import { renderPackPanel } from "./pack-panel.js";
import { CREATE_COMMAND } from "./rules.js";

export const ASK = [["ask", "Ask about it"], ["check", "Change what it checks"], ["scope", "Change where it applies"], ["delivery", "Change what happens"], ["delete", "Delete it"]];
const ASK_OPEN = { check: true, scope: true, delivery: true };

export function askPrompt(stored, k) {
  const ref = `the Rerail rule "${stored.rule.id}" (${titleOf(stored)})`;
  if (k === "check") return `Update ${ref}. Show me the change before saving. I want it to check: `;
  if (k === "scope") return `Update ${ref}. Currently it applies to: ${scopeText(stored)} Show me the change before saving. I want it to apply to: `;
  if (k === "delivery") return `Update ${ref}. Currently, on a violation: ${modeLabel(stored)}. Show me the change before saving. Instead I want it to: `;
  if (k === "delete") return `Delete ${ref}. Show me what will be removed and ask me to confirm first.`;
  return `Explain ${ref}: what it checks, where it applies, and why it logged its most recent violation.`;
}

const kv = rows => rows.flatMap(([k, v, cls]) => [h("dt", null, k), h("dd", { class: cls }, v)]);
const MODES = [["log", "Log", "logged"], ["steer", "Steer", "steered"], ["block", "Block", "blocked"]];

/* ---------- rule ---------- */

let p = null;

function buildRule() {
  const el = {};
  el.back = h("button.back", { "data-act": "panelback" }, "← Back to list");
  el.eyebrow = h("div.eyebrow");
  el.title = h("h2");
  el.sw = h("button.sw", { role: "switch", "aria-checked": "false", "data-act": "toggle" });
  el.swText = h("span");
  el.ask = h("button.btn.sm.solid", { "data-act": "asktoggle", "aria-expanded": "false" }, "Edit with coding agent");
  el.askChips = h("div.chips", null, ASK.map(([k, l]) => h("button.chip", { "data-act": "askintent", "data-i": k, "aria-pressed": "false" }, l)));
  el.askText = h("textarea", { id: "askText", readonly: true, "aria-label": "Prompt for your coding agent" });
  el.askFoot = h("span");
  el.pask = h("div.pask", { hidden: true },
    h("p.lbl", null, "What do you want to do?"), el.askChips, el.askText,
    h("div.foot", null, el.askFoot, h("button.btn.sm.solid", { "data-act": "askcopy" }, "Copy prompt")));
  el.chips = h("div.pchips", null, h("span.badge.origin"), h("span.badge.where"));
  el.modes = h("div.rangectl.modes", { role: "group", "aria-label": "What Rerail does on a violation" },
    MODES.map(([v, l, o]) => h("button", { "data-v": v, disabled: true, "aria-pressed": "false" }, icon(o, 14), l)));
  el.modeText = h("p");
  el.stats = h("div.pstats", null, h("div", null, h("b.num"), h("span")), h("div", null, h("b"), h("span", null, "last logged")), h("div", null, h("b.num"), h("span", null, "since install")));
  el.condition = h("div");
  el.violations = h("section.psec", null, h("h3", null, "Violations"), h("p.fires"));
  el.from = h("dl.kv");
  el.applies = h("p");
  el.sentHead = h("h3");
  el.sent = h("p");
  el.recent = h("div");
  el.techRows = h("dl.kv", { style: "margin-top:8px" });
  el.techPre = h("pre");
  el.body = h("div.pbody", null, el.chips,
    h("div.enf", null, h("div.lbl", null, "On a violation"), el.modes, el.modeText),
    el.stats,
    h("section.psec", null, h("h3", null, "Condition"), el.condition),
    el.violations,
    h("section.psec", null, h("h3", null, "Where it came from"), el.from),
    h("section.psec", null, h("h3", null, "Where it applies"), el.applies),
    h("section.psec", null, el.sentHead, el.sent),
    h("section.psec.sep", null, h("h3", null, "Recent activity"), el.recent),
    h("details", null, h("summary", null, "Technical"), h("div.in", null, el.techRows, el.techPre)));
  el.root = h("div.pwrap", null,
    h("div.phead", null,
      h("div.ptop", null, h("div", null, el.back, el.eyebrow, el.title), h("button.x", { "data-act": "close", "aria-label": "Close" }, "✕")),
      h("div.pact", null, el.sw, el.swText, h("span.grow"), el.ask, el.pask)),
    el.body);
  p = el;
}

function renderAsk(stored) {
  setAttr(p.ask, "aria-expanded", String(ui.askOpen));
  setAttr(p.pask, "hidden", !ui.askOpen);
  if (!ui.askOpen) return;
  for (const c of p.askChips.children) setAttr(c, "aria-pressed", String(c.dataset.i === ui.askIntent));
  const prompt = askPrompt(stored, ui.askIntent);
  if (p.askText.value !== prompt) p.askText.value = prompt;
  setText(p.askFoot, `Paste into a coding agent session${ASK_OPEN[ui.askIntent] ? ", then finish the sentence" : ""}.`);
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
  setText(s2.firstChild, all && lastSeen ? whenLabel(lastSeen) : "–");
  setText(s3.firstChild, measured ? fmt(all) : "–");
}

function exampleCode(example) {
  return h("code", { title: example.path ?? null }, example.text.trimEnd());
}

function renderCondition(stored) {
  const examples = packRuleOf(stored.rule.id)?.rule.examples;
  const text = conditionText(stored) || `${titleOf(stored)}.`;
  setContent(p.condition, `${stored.rule.id}|${stored.revision}|${examples ? 1 : 0}`, () => [
    h("p", null, text),
    examples ? h("dl.cp", null, h("dt", null, "Caught"), h("dd", null, exampleCode(examples.caught)),
      h("dt", null, "Passes"), h("dd", null, exampleCode(examples.passes))) : null,
  ]);
}

const lineOf = x => (x.span ? `:${x.span.startLine + 1}` : "");

function renderViolations(id) {
  const inRange = ruleFindings(id);
  setAttr(p.violations, "hidden", inRange.length === 0);
  if (inRange.length === 0) return;
  const dirs = new Map();
  for (const x of inRange) {
    if (!x.path?.includes("/")) continue;
    const dir = x.path.split("/").slice(0, -1).join("/");
    dirs.set(dir, (dirs.get(dir) ?? 0) + 1);
  }
  const top = [...dirs].sort((a, b) => b[1] - a[1])[0];
  const sessions = new Set(inRange.map(x => x.sessionId)).size;
  setContent(p.violations.lastChild, `${inRange.length}|${top}|${sessions}|${ui.range}`, () => [
    h("b", null, fmt(inRange.length)), ` ${rangeLabel()}`,
    top ? [" · ", h("b", null, fmt(top[1])), " in ", h("span.mono", null, top[0])] : null,
    " · ", h("b", null, fmt(sessions)), ` ${plural(sessions, "session")}`,
  ]);
}

/** Why a By Rerail rule is present: the pack it is added through (and why that pack fits), or on its own. */
function whyOn(stored, pack) {
  if (attachedPackOf(stored) === undefined) return "Its rule pack is not added here; the rule is added to this repository on its own.";
  const recommended = data.recommendations?.find(r => r.packId === pack.id);
  return `The rule pack ${recommended?.reason ?? pack.reason}.`;
}

function renderOrigin(stored) {
  const id = stored.rule.id, pack = packOfRule(stored), group = packRuleOf(id)?.rule.group;
  const checkoutPath = data.checkout?.checkoutPath;
  const folders = stored.folders.map(folderText).join(", ");
  // Folders that disagree each show their own on and mode; switching the rule then sets it in every folder.
  const on = stored.mixed
    ? stored.folderStates.map(state => `${state.on ? "On" : "Off"} in ${folderText(state.folder)}`).join(", ")
    : `${stored.enabled ? "On" : "Off"} · ${deciderText(stored.onSource, checkoutPath)}`;
  const mode = stored.mixed
    ? stored.folderStates.map(state => `${MODE_LABEL[state.mode] ?? state.mode} in ${folderText(state.folder)}`).join(", ")
    : `${modeLabel(stored)} · ${deciderText(stored.modeSource, checkoutPath)}`;
  const why = pack ? whyOn(stored, pack) : "";
  setContent(p.from, JSON.stringify([id, pack?.id, why, folders, on, mode]), () => kv([
    ["Origin", originLabel(stored)],
    ...(pack ? [["Rule pack", group ? `${pack.name} · ${group}` : pack.name], ["Why it is on", why]] : []),
    ...(isByRerail(stored) && !pack ? [["Rule pack", "None: it is added to this repository on its own"]] : []),
    ["Added at", folders],
    ["On", on],
    ["Mode", mode],
    ["Rule id", id, "mono"],
  ]));
}

function renderSent(stored, id) {
  setText(p.sentHead, isLog(stored) ? "Recorded as" : "Sent to the coding agent as");
  const message = ruleFindings(id)[0]?.message ?? messageText(stored);
  setContent(p.sent, message, () => h("span.said2", null, message));
}

function renderRecent(id) {
  const list = ruleFindings(id), top = list.slice(0, 3), all = allFires(id);
  const host = sessionId => data.sessions.find(s => s.sessionId === sessionId)?.host;
  setContent(p.recent, `${id}|${top.map(x => `${x.auditId}:${x.id}`).join(",")}|${list.length}|${all}|${ui.range}`, () => {
    if (top.length === 0) return h("p.muted", null, all ? `None logged ${rangeLabel()}. Widen the range to see earlier ones.` : "No violations logged yet. Rerail is silent until this rule matches.");
    return [h("div.ract", null, top.map(x => h("div.rrow", null,
      h("span.when", null, `${whenLabel(x.at)} · ${clock(x.at)}`),
      h("div.what", null, x.path ? h("span.mono", null, `${x.path}${lineOf(x)}`) : h("span.muted", null, "no path recorded"),
        h("button.open", { "data-act": "session", "data-i": x.sessionId }, shortSessionId(x.sessionId, host(x.sessionId))))))),
    h("button.btn.sm.ghost.link", { "data-act": "actrule", "data-i": id, style: "margin-top:6px" }, list.length > 3 ? `See all ${fmt(list.length)} in Activity` : "See it in Activity")];
  });
}

function renderTechnical(stored, id) {
  const tech = technical(stored);
  setContent(p.techRows, `${id}|${stored.revision}`, () => kv(tech.rows));
  setText(p.techPre, tech.pattern);
  setAttr(p.techPre, "hidden", tech.pattern === "");
}

function renderRule(root, stored) {
  if (p === null) buildRule();
  if (p.root.parentNode !== root) { root.replaceChildren(p.root); delete root.dataset.sig; }
  const id = stored.rule.id, pack = attachedPackOf(stored), group = packRuleOf(id)?.rule.group;
  setText(p.back, ui.backToPack ? `← Back to ${data.packs.find(x => x.id === ui.backToPack)?.name ?? "the rule pack"}` : "← Back to list");
  setAttr(p.back, "class", ui.backToPack ? "back show" : "back");
  setText(p.eyebrow, kindLabel(stored));
  setText(p.title, titleOf(stored));
  setAttr(p.sw, "aria-checked", String(stored.enabled));
  setAttr(p.sw, "aria-label", `Turn ${titleOf(stored)} ${stored.enabled ? "off" : "on"}`);
  setAttr(p.sw, "data-i", id);
  setText(p.swText, stored.enabled ? "On" : "Off");
  setAttr(p.ask, "data-i", id);
  renderAsk(stored);
  const [origin, where] = p.chips.children;
  setAttr(origin, "class", `badge origin ${isByRerail(stored) ? "rerail" : "mine"}`);
  setText(origin, originChip(stored));
  setText(where, pack ? (group ? `${pack.name} · ${group}` : pack.name) : isByRerail(stored) ? "Other rules by Rerail" : "Created rules");
  renderModes(stored);
  renderStats(id);
  renderCondition(stored);
  renderViolations(id);
  renderOrigin(stored);
  setText(p.applies, scopeText(stored));
  renderSent(stored, id);
  renderRecent(id);
  renderTechnical(stored, id);
}

/* ---------- create a rule (W2) ---------- */

function commandBlock(command) {
  return h("div.cmd", null, h("code", null, command), h("button.btn.sm.solid", { "data-act": "cmdcopy", "data-i": command }, icon("copy"), "Copy"));
}

function renderCreate(root) {
  setContent(root, `create|${repoName()}`, () => h("div.pwrap", null,
    h("div.phead", null, h("div.ptop", null, h("div", null, h("div.eyebrow", null, "Created rules"), h("h2", null, "Create a rule")),
      h("button.x", { "data-act": "close", "aria-label": "Close" }, "✕"))),
    h("div.pbody", null,
      h("p.pintro", null, "You create rules with your coding agent, in your own words. Type this in a session in ", h("span.mono", null, repoName() || "this repo"), ", a new one or one you already have open:"),
      commandBlock(CREATE_COMMAND),
      h("p.pintro", null, "It asks what the rule should look for, where it applies and what happens on a violation. Then it writes the rule and tells you its id."),
      h("p.pintro", null, "Your rule shows here under Created rules, on by default, with every violation it logs or steers from the first session on."))));
}

/** Render whichever panel `ui.panel` names, and mark the page so the grid makes room for it. */
export function renderPanel(root) {
  const panel = ui.panel;
  const stored = panel?.kind === "rule" ? data.rules.get(panel.id) : undefined;
  const pack = panel?.kind === "pack" ? data.packs.find(x => x.id === panel.id) : undefined;
  const open = stored !== undefined || pack !== undefined || panel?.kind === "create";
  const state = open ? "open" : "";
  if (document.body.dataset.panel !== state) document.body.dataset.panel = state;
  if (!open) return;
  if (stored) renderRule(root, stored);
  else if (pack) renderPackPanel(root, pack);
  else renderCreate(root);
}

/** Scroll the body back to the top when a different rule opens. */
export function resetPanelScroll() {
  if (p !== null) p.body.scrollTop = 0;
}
