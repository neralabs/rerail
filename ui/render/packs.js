/**
 * WP: every rule pack that comes with Rerail, added or not, with Add pack and Remove pack.
 * WR (`#rule-packs?review=1`): the packs set-up picked for this repository, each with why and a
 * switch, before Rerail starts. The person starts Rerail back in the terminal or coding agent.
 */
import { h, setContent, icon } from "../dom.js";
import { fmt, plural } from "../format.js";
import { isRunning } from "../derive.js";
import { data, ui, isAttached, packGroups, repoName, stateHere, isSetUp, keptPacks, runningCount } from "../state.js";
import { packAction, packIcon, packKind, startModeText } from "./pack-panel.js";

/** "3 groups · 8 rules", plus how many are on once Rerail is started here: before that, nothing is on. */
function packMeta(pack) {
  const groups = packGroups(pack).length;
  const on = pack.rules.map(r => data.rules.get(r.id)).filter(stored => stored !== undefined && isRunning(stored)).length;
  const counted = isAttached(pack) && stateHere() === "started";
  return `${fmt(groups)} ${plural(groups, "group")} · ${fmt(pack.rules.length)} ${plural(pack.rules.length, "rule")}${counted ? ` · ${fmt(on)} on` : ""}`;
}

const addedMark = () => h("span.added", null, icon("check"), "Added");

function catalogItem(pack) {
  const status = isAttached(pack) && ui.removeAsk !== pack.id ? addedMark() : null;
  if (ui.catView === "cards") {
    return h("div.pcard", { "data-act": "viewpack", "data-i": pack.id, role: "button", tabindex: "0" },
      h("div.pc-top", null, packIcon(pack), h("div", null, h("b", null, pack.name), h("span.kind", null, packKind(pack)))),
      h("span.sub", null, pack.covers), h("span.meta", null, packMeta(pack)),
      h("div.pc-foot", null, h("span.pstatus", null, status), packAction(pack)));
  }
  return h("div.prow2", { "data-act": "viewpack", "data-i": pack.id, role: "button", tabindex: "0" },
    packIcon(pack), h("div.pt2", null, h("b", null, pack.name), h("span.sub", null, pack.covers), h("span.meta", null, packMeta(pack))),
    h("div.pstatus", null, status), h(`div.pact2${ui.removeAsk === pack.id ? ".ask" : ""}`, null, packAction(pack)));
}

function catalog() {
  const shown = data.packs;
  const added = shown.filter(isAttached).length;
  const repo = repoName() || "this repo";
  return h("div.catalog", null,
    h("div.cathead", null, h("button.btn.sm.ghost.link", { "data-act": "backrules" }, icon("back"), "Rules")),
    h("div.ghead", null, h("h2", null, "Rule packs", h("span.n2", null, `${fmt(added)} of ${fmt(shown.length)} added`)),
      h("p", null, `Each rule pack is a set of rules that come with Rerail, filed in smaller groups. Each repo has its own rule packs: these are for ${repo}. Added rules file under their rule pack on Rules.`)),
    h("div.cattools", null,
      h("div.rangectl.viewctl", { role: "group", "aria-label": "View" },
        h("button", { "data-act": "catview", "data-i": "list", "aria-pressed": String(ui.catView === "list") }, "List"),
        h("button", { "data-act": "catview", "data-i": "cards", "aria-pressed": String(ui.catView === "cards") }, "Cards"))),
    ui.catView === "cards" ? h("div.cards", null, shown.map(catalogItem)) : h("div.list", null, shown.map(catalogItem)));
}

/* ---------- WR ---------- */

function reviewRow(pack, reason) {
  const kept = keptPacks().includes(pack.id);
  return h("div.prow2.review", { "data-act": "viewpack", "data-i": pack.id, role: "button", tabindex: "0" },
    packIcon(pack),
    h("div.pt2", null, h("b", null, pack.name), h("span.sub", null, reason ? `Picked because it ${reason}.` : pack.covers), h("span.meta", null, packMeta(pack))),
    h("div.pstatus", null, kept ? "On" : reason ? "Switched off" : ""),
    h("div.pact2", null, reason || kept
      ? h("button.sw", { role: "switch", "aria-checked": String(kept), "aria-label": `${kept ? "Switch off" : "Keep"} ${pack.name}`, "data-act": "reviewtoggle", "data-i": pack.id })
      : h("button.btn.sm", { "data-act": "reviewtoggle", "data-i": pack.id }, icon("plus"), "Add pack")));
}

function review() {
  const repo = repoName() || "this repo", state = stateHere();
  if (data.recommendations === null) return h("div.catalog", null, h("div.list", null, h("div.empty", null, h("p", null, `Reading ${repo}…`))));
  const picked = data.recommendations.map(r => ({ pack: data.packs.find(p => p.id === r.packId), reason: r.reason })).filter(x => x.pack);
  const others = data.packs.filter(p => !picked.some(x => x.pack.id === p.id));
  const kept = keptPacks(), keptRules = data.packs.filter(p => kept.includes(p.id)).reduce((n, p) => n + p.rules.length, 0);
  const started = state === "started";
  return h("div.catalog.review", null,
    h("div.ghead", null, h("h2", null, "Review before starting", h("span.n2", null, `${fmt(picked.length)} rule ${plural(picked.length, "pack")} picked for ${repo}`)),
      h("p", null, `Rerail matched the files in this repo against the rule packs it comes with. Each rule pack below says why it was picked. Nothing is on until you start Rerail. ${startModeText(picked.map(x => x.pack))}`)),
    h("div.welcome.next", { role: "status" }, icon("done", 22),
      h("div", null, started
        ? [h("b", null, `Rerail is on in ${repo}.`), h("p", null, "Changes here apply straight away.")]
        : [h("b", null, `${fmt(kept.length)} rule ${plural(kept.length, "pack")} kept, with ${fmt(keptRules)} ${plural(keptRules, "rule")}.`),
          h("p", null, "Switch off any rule pack you don’t want; each change saves as you make it. Then go back to your terminal or coding agent to start Rerail.")])),
    h("div.list", null, picked.map(({ pack, reason }) => reviewRow(pack, reason))),
    others.length ? h("div.ghead.top", null, h("h2", null, "Other rule packs", h("span.n2", null, `${fmt(others.length)} not picked`)),
      h("p", null, "They come with Rerail and are off. Add one if it suits how your team works.")) : null,
    others.length ? h("div.list", null, others.map(pack => reviewRow(pack, null))) : null);
}

export function renderPacks(root, view) {
  const sig = JSON.stringify([view, data.packs.map(p => p.id), [...data.attachedPacks], isSetUp(), data.recommendations, ui.catView, ui.removeAsk,
    repoName(), stateHere(), runningCount()]);
  setContent(root, sig, () => (view === "review" ? review() : catalog()));
}
