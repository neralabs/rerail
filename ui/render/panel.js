import { h, setAttr, setText, setContent } from "../dom.js";
import { fmt, plural, whenLabel, stamp } from "../format.js";
import { originOf, ruleTitle, kindLabel, modeLabel, isLog, scopeText, technical } from "../derive.js";
import { data, ui, rangeLabel, fires, allFires, rerails, isMeasured, activityAllOf, ruleFindings } from "../state.js";

export const ASK = [["ask", "Ask about it"], ["check", "Change what it checks"], ["scope", "Change where it applies"], ["mode", "Change what happens"], ["delete", "Delete it"]];
const ASK_OPEN = { check: true, scope: true, mode: true };

export function askPrompt(stored, k) {
  const ref = `the rule "${stored.rule.id}" (${ruleTitle(stored)})`;
  if (k === "check") return `Using Nera, update ${ref}. Show me the change before saving. I want it to check: `;
  if (k === "scope") return `Using Nera, update ${ref}. Currently it applies to: ${scopeText(stored)} Show me the change before saving. I want it to apply to: `;
  if (k === "mode") return `Using Nera, update ${ref}. Currently: ${modeLabel(stored)}. Show me the change before saving. Instead I want it to: `;
  if (k === "delete") return `Using Nera, delete ${ref}. Show me what will be removed and ask me to confirm first.`;
  return `Using Nera, explain ${ref}: what it checks, where it applies, and why it triggered most recently.`;
}

let p = null;

function build(root) {
  const el = {};
  el.back = h("button.back", { "data-act": "close" }, "← Back to list");
  el.eyebrow = h("div.eyebrow");
  el.title = h("h2");
  el.close = h("button.x", { "data-act": "close", "aria-label": "Close" }, "✕");
  el.sw = h("button.sw", { role: "switch", "aria-checked": "false", "aria-label": "Toggle", "data-act": "toggle" });
  el.swText = h("span");
  el.ask = h("button.btn.sm.solid", { "data-act": "asktoggle", "aria-expanded": "false" }, "Edit with agent");
  el.askChips = h("div.chips", null, ASK.map(([k, l]) => h("button.chip", { "data-act": "askintent", "data-i": k, "aria-pressed": "false" }, l)));
  el.askText = h("textarea", { id: "askText", readonly: true, "aria-label": "Prompt for your agent" });
  el.askFoot = h("span");
  el.pask = h("div.pask", { hidden: true },
    h("p.lbl", null, "What do you want to do?"), el.askChips, el.askText,
    h("div.foot", null, el.askFoot, h("button.btn.sm.solid", { "data-act": "askcopy" }, "Copy prompt")));
  el.chips = h("div.pchips", null, h("span.badge.origin"), h("span.badge.mode"));
  el.lead = h("p.plead");
  el.stats = h("div.pstats", null,
    h("div", null, h("b.num"), h("span")), h("div", null, h("b"), h("span")), h("div", null, h("b.num"), h("span", null, "since install")));
  el.from = h("dl.kv");
  el.applies = h("p");
  el.happens = h("p");
  el.techRows = h("dl.kv", { style: "margin-top:8px" });
  el.techPre = h("pre");
  el.stamp = h("p.stamp");
  root.replaceChildren(
    h("div.phead", null,
      h("div.ptop", null, h("div", null, el.back, el.eyebrow, el.title), el.close),
      h("div.pact", null, el.sw, el.swText, h("span.grow"), el.ask, el.pask)),
    h("div.pbody", null, el.chips, el.lead, el.stats,
      h("section.psec", null, h("h3", null, "Where it came from"), el.from),
      h("section.psec", null, h("h3", null, "Where it applies"), el.applies),
      h("section.psec", null, h("h3", null, "What happens when it triggers"), el.happens),
      h("details", null, h("summary", null, "Technical"), h("div.in", null, el.techRows, el.techPre)),
      el.stamp));
  el.body = root.querySelector(".pbody");
  p = el;
}

const kv = rows => rows.flatMap(([k, v, cls]) => [h("dt", null, k), h("dd", { class: cls }, v)]);

function renderAsk(stored) {
  setAttr(p.ask, "aria-expanded", String(ui.askOpen));
  setAttr(p.pask, "hidden", !ui.askOpen);
  if (!ui.askOpen) return;
  for (const c of p.askChips.children) setAttr(c, "aria-pressed", String(c.dataset.i === ui.askIntent));
  const prompt = askPrompt(stored, ui.askIntent);
  if (p.askText.value !== prompt) p.askText.value = prompt;
  setText(p.askFoot, `Paste into your agent session${ASK_OPEN[ui.askIntent] ? ", then finish the sentence" : ""}.`);
}

function renderStats(id, measured, rr) {
  const f = fires(id), all = allFires(id), lastSeen = activityAllOf(id).lastSeenAt;
  const [s1, s2, s3] = p.stats.children;
  setText(s1.firstChild, measured ? fmt(f) : "—");
  setText(s1.lastChild, measured ? `${plural(f, "trigger")} ${rangeLabel()}` : "not measured");
  setText(s2.firstChild, lastTriggered(all, lastSeen, measured));
  setText(s2.lastChild, rr ? "last re-rail" : "last triggered");
  setText(s3.firstChild, measured ? fmt(all) : "—");
}

function lastTriggered(all, lastSeen, measured) {
  if (all && lastSeen) return whenLabel(lastSeen);
  return measured ? "never" : "—";
}

function renderOrigin(stored, id, ob) {
  setContent(p.from, `${id}|${ob.label}|${stored.applicability}`, () => kv([
    ["Origin", ob.label],
    ["Applies", stored.applicability === "project" ? "This project" : "Every project"],
    ["Rule id", id, "mono"],
  ]));
}

function renderHappens(stored, id, rr) {
  const message = stored.kind === "conversation" ? stored.rule.action.content : ruleFindings(id)[0]?.message ?? stored.rule.clauses[0]?.violation?.message ?? "the rule's message";
  setContent(p.happens, `${modeLabel(stored)}|${isLog(stored)}|${rr}|${ui.range}|${message}`, () => [
    `${modeLabel(stored)}. `,
    rr ? `Steered the agent ${fmt(rr)} ${plural(rr, "time")} ${rangeLabel()}. ` : "",
    isLog(stored) ? "Nothing is sent to the agent. Recorded as: " : "The agent sees: ",
    h("span.said2", null, message),
  ]);
}

function renderTechnical(stored, id) {
  const tech = technical(stored);
  setContent(p.techRows, `${id}|${stored.revision}|${stored.updatedAt}`, () => kv(tech.rows));
  setText(p.techPre, tech.pattern);
  setAttr(p.techPre, "hidden", tech.pattern === "");
  setContent(p.stamp, `${stored.createdAt}|${stored.updatedAt}`, () => [`Created ${stamp(stored.createdAt)}`, h("br"), `Updated ${stamp(stored.updatedAt)}`]);
}

export function renderPanel(root) {
  if (p === null) build(root);
  const stored = ui.selectedRule === null ? undefined : data.rules.get(ui.selectedRule);
  const open = stored !== undefined;
  const panelState = open ? "open" : "";
  if (document.body.dataset.panel !== panelState) document.body.dataset.panel = panelState;
  if (!open) return;
  const id = stored.rule.id, rr = rerails(id), title = ruleTitle(stored), ob = originOf(stored);
  setText(p.eyebrow, kindLabel(stored));
  setText(p.title, title);
  setAttr(p.sw, "aria-checked", String(stored.enabled));
  setAttr(p.sw, "data-i", id);
  setText(p.swText, stored.enabled ? "On" : "Off");
  setAttr(p.ask, "data-i", id);
  renderAsk(stored);
  const [origin, mode] = p.chips.children;
  setAttr(origin, "class", `badge ${ob.cls}`); setText(origin, ob.label);
  setText(mode, modeLabel(stored));
  setText(p.lead, `${title}.`);
  renderStats(id, isMeasured(id), rr);
  renderOrigin(stored, id, ob);
  setText(p.applies, scopeText(stored));
  renderHappens(stored, id, rr);
  renderTechnical(stored, id);
}

/** Scroll the body back to the top when a different rule opens. */
export function resetPanelScroll() {
  if (p !== null) p.body.scrollTop = 0;
}
