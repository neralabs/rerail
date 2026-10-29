/**
 * Add rule packs (`#packs`; `#rule-packs` before v8): every rule pack that comes with Rerail, alphabetical, filtered
 * by kind, as a list or as cards; each added or not, with Add pack or a quiet Remove pack that asks first. A pack is
 * added whole: its rules arrive as the pack starts them, opt-in ones off. The review (`#packs?review=1`, set-up's
 * `#rule-packs?review=1`): the packs set-up picked for this repository, each with why and a switch, then the others
 * with Add pack. The person starts Rerail back in the terminal or coding agent.
 */
import { h, setContent, icon } from "../dom.js";
import { fmt, plural } from "../format.js";
import { isRunning } from "../derive.js";
import { data, ui, isAttached, packGroups, repoName, stateHere, isSetUp, keptPacks, runningCount } from "../state.js";
import { loadRecommendations } from "../poll.js";
import { invalidate, toast } from "../shell.js";
import { keepPack } from "../actions.js";
import { catalogKinds, catalogPacks } from "../pack-model.js";
import { packAction, packIcon, packKind } from "../render/pack-panel.js";

export const needs = ["rules", "packs", "activity"];

/** The catalog's kind filter ("all" or a pack category); back to All each time the catalog is entered. */
let kind = "all";

/** The packs the catalog and the review list: the ones with rules to add. */
const listed = () => data.packs.filter(pack => pack.rules.length > 0);

/** "8 rules", plus how many are on once it is added and Rerail is started here: before that, nothing is on. */
function packMeta(pack) {
  const n = pack.rules.length;
  const counted = isAttached(pack) && stateHere() === "started";
  const on = counted ? pack.rules.map(r => data.rules.get(r.id)).filter(stored => stored !== undefined && isRunning(stored)).length : 0;
  return `${fmt(n)} ${plural(n, "rule")}${counted ? ` · ${fmt(on)} on` : ""}`;
}

function catalogItem(pack) {
  const asking = ui.removeAsk === pack.id;
  const status = isAttached(pack) && !asking ? h("span.added", null, icon("check"), "Added") : null;
  const action = packAction(pack, { quiet: true });
  if (ui.catView === "cards") {
    return h("div.pcard", { "data-act": "viewpack", "data-i": pack.id, role: "button", tabindex: "0" },
      h("div.pc-top", null, packIcon(pack), h("div.pc-name", null, h("b", null, pack.name), h("span.kind", null, packKind(pack))),
        status ? h("span.pstatus", null, status) : null),
      h("span.sub", null, pack.covers),
      h("div.pc-foot", null, h("span.meta", null, packMeta(pack)), action));
  }
  return h("div.prow2", { "data-act": "viewpack", "data-i": pack.id, role: "button", tabindex: "0" },
    packIcon(pack), h("div.pt2", null, h("b", null, pack.name), h("span.sub", { title: pack.covers }, pack.covers), h("span.meta", null, packMeta(pack))),
    h(`div.pend${asking ? ".ask" : ""}`, null, asking ? null : h("span.pstatus", null, status), action));
}

function catalog() {
  const all = listed(), kinds = catalogKinds(all);
  if (!kinds.some(([k]) => k === kind)) kind = "all";
  const shown = catalogPacks(all, kind);
  const added = shown.filter(isAttached).length;
  return h("div.catalog", null,
    h("div.ghead", null, h("h2", null, "Rule packs", h("span.n2", null, `${fmt(added)} of ${fmt(shown.length)} added`)),
      h("p", null, "Each pack checks one language or framework. Added packs get their own page in the menu.")),
    h("div.cattools", null,
      h("div.chips", { role: "group", "aria-label": "Pack kind" },
        kinds.map(([k, label]) => h("button.chip", { "data-act": "catkind", "data-i": k, "aria-pressed": String(kind === k) }, label))),
      h("div.rangectl.viewctl", { role: "group", "aria-label": "View" },
        h("button", { "data-act": "catview", "data-i": "list", "aria-pressed": String(ui.catView === "list") }, "List"),
        h("button", { "data-act": "catview", "data-i": "cards", "aria-pressed": String(ui.catView === "cards") }, "Cards"))),
    ui.catView === "cards" ? h("div.cards", null, shown.map(catalogItem)) : h("div.list", null, shown.map(catalogItem)));
}

/* ---------- the review ---------- */

function reviewRow(pack, reason) {
  const kept = keptPacks().includes(pack.id), groups = packGroups(pack).length, n = pack.rules.length, repo = repoName() || "this repo";
  return h("div.prow2", { "data-act": "viewpack", "data-i": pack.id, role: "button", tabindex: "0" },
    packIcon(pack),
    h("div.pt2", null, h("b", null, pack.name), h("span.sub", { title: pack.covers }, pack.covers),
      reason === null ? null : h("span.why", null, h("b", null, "Why"), reason),
      h("span.meta", null, `${fmt(groups)} ${plural(groups, "group")} · ${fmt(n)} ${plural(n, "rule")}`)),
    h("div.pact2", null, reason !== null || kept
      ? h("span.swl", null, h("span", null, kept ? "On" : "Off"),
        h("button.sw", { role: "switch", "aria-checked": String(kept), "aria-label": `${pack.name} in ${repo}`, "data-act": "reviewtoggle", "data-i": pack.id }))
      : h("button.btn.sm", { "data-act": "reviewtoggle", "data-i": pack.id }, icon("plus"), "Add pack")));
}

function review() {
  const repo = repoName() || "this repo";
  if (data.recommendations === null) return h("div.catalog.rv", null, h("div.list", null, h("div.empty", null, h("p", null, `Reading ${repo}…`))));
  const packs = catalogPacks(listed(), "all");
  const reasons = new Map(data.recommendations.map(r => [r.packId, r.reason ?? ""]));
  const picked = packs.filter(p => reasons.has(p.id)), others = packs.filter(p => !reasons.has(p.id));
  const kept = keptPacks(), keptPicked = packs.filter(p => kept.includes(p.id));
  const keptRules = keptPicked.reduce((n, p) => n + p.rules.length, 0);
  return h("div.catalog.rv", null,
    h("div.cathead", null, h("button.btn.sm.ghost.link", { "data-act": "nav", "data-i": "overview" }, icon("back"), repo)),
    h("div.ghead", null, h("h2", null, `Review the rule packs for ${repo}`,
      h("span.n2", null, `${fmt(keptPicked.length)} ${plural(keptPicked.length, "pack")} · ${fmt(keptRules)} ${plural(keptRules, "rule")}`)),
    h("p", null, "Picked for ", h("span.mono", null, repo), " from its files. Switch off any you don't want, then finish in your coding agent.")),
    h("div.list", null, picked.map(pack => reviewRow(pack, reasons.get(pack.id)))),
    others.length ? h("div.ghead.top", null, h("h2", null, "Other rule packs", h("span.n2", null, `${fmt(others.length)} not picked`)),
      h("p", null, "Not picked for this repo. Add any that fit.")) : null,
    others.length ? h("div.list", null, others.map(pack => reviewRow(pack, null))) : null);
}

export function render(root) {
  const view = ui.view;
  const sig = JSON.stringify([view, kind, data.packs.map(p => p.id), [...data.attachedPacks.keys()], isSetUp(), data.recommendations, ui.catView, ui.removeAsk,
    repoName(), stateHere(), runningCount()]);
  setContent(root, sig, () => (view === "review" ? review() : catalog()));
}

/** The catalog opens on every kind; the review reads the repository for the packs set-up would pick, once per repository. */
export function enter() {
  if (ui.view === "packs") kind = "all";
  if (ui.view !== "review" || ui.scope === null || data.recommendations !== null) return;
  loadRecommendations().then(invalidate, error => toast(`Could not read the repo: ${error.message}`));
}

export const actions = {
  catkind: k => { kind = k; invalidate(); },
  catview: v => { ui.catView = v; invalidate(); },
  reviewtoggle: id => keepPack(id, !keptPacks().includes(id)),
};
