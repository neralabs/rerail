/**
 * Add rule packs (`#packs`; `#rule-packs` before v8): every rule pack installed on this machine, alphabetical, filtered
 * by kind, as a list or as cards; each added or not, with Add pack or a quiet Remove pack that asks first, its
 * question on a line of its own under the pack's text. A pack removed here reads Removed with Undo until the page is
 * left. A pack is added whole: its rules arrive as the pack starts them, opt-in ones off. The review
 * (`#packs?review=1`, set-up's `#rule-packs?review=1`): the packs set-up picked for this repository, each with why
 * and a switch, then the others with Add pack. The person starts Rerail back in the terminal or coding agent.
 */
import { h, setContent, icon } from "../dom.js";
import { failed, fmt, plural } from "../format.js";
import { isRerailPack, isRunning, isWorktreePack, publisherLabel } from "../derive.js";
import { data, ui, isAttached, repoName, stateHere, isSetUp, keptPacks, runningCount } from "../state.js";
import { loadRecommendations } from "../poll.js";
import { invalidate, toast } from "../shell.js";
import { commonActions, keepPack } from "../actions.js";
import { catalogKinds, catalogPacks } from "../pack-model.js";
import { packAction, packIcon, packKind, removePackControl } from "../render/pack-panel.js";

export const needs = ["rules", "packs", "activity"];

/** The catalog's kind filter ("all" or a pack category); back to All each time the catalog is entered. */
let kind = "all";

/** The packs the catalog and the review list: the installed ones with rules to add, never the worktree's own rules. */
const listed = () => data.packs.filter(pack => pack.rules.length > 0 && !isWorktreePack(pack));

/** "8 rules", plus how many are on once it is added and Rerail is started here: before that, nothing is on. */
function packMeta(pack) {
  const n = pack.rules.length;
  const counted = isAttached(pack) && stateHere() === "started";
  const on = counted ? pack.rules.map(r => data.rules.get(r.id)).filter(stored => stored !== undefined && isRunning(stored)).length : 0;
  return `${fmt(n)} ${plural(n, "rule")}${counted ? ` · ${fmt(on)} on` : ""}`;
}

/** Where a pack stands in the catalog: Added, Removed during this visit (with Undo), or neither. */
function catalogState(pack) {
  if (isAttached(pack)) return { status: h("span.added", null, icon("check"), "Added"), action: packAction(pack, { quiet: true }) };
  if (ui.removedPacks.has(pack.id)) {
    return { status: h("span.rmtag", null, "Removed"), action: h("button.btn.sm", { "data-act": "undoremove", "data-i": pack.id, title: `Add ${pack.name} back` }, "Undo") };
  }
  return { status: null, action: packAction(pack, { quiet: true }) };
}

function catalogItem(pack) {
  const asking = ui.removeAsk === pack.id;
  const { status, action } = asking ? { status: null, action: null } : catalogState(pack);
  const question = asking ? h("div.rmline", null, removePackControl(pack)) : null;
  if (ui.catView === "cards") {
    return h("div.pcard", { "data-act": "viewpack", "data-i": pack.id, role: "button", tabindex: "0" },
      h("div.pc-top", null, packIcon(pack), h("div.pc-name", null, h("b", null, pack.name), h("span.kind", null, packKind(pack))),
        status ? h("span.pstatus", null, status) : null),
      h("span.sub", null, pack.covers),
      question,
      h("div.pc-foot", null, h("span.meta", null, packMeta(pack)), action));
  }
  return h("div.prow2", { "data-act": "viewpack", "data-i": pack.id, role: "button", tabindex: "0" },
    packIcon(pack), h("div.pt2", null, h("b", null, pack.name), h("span.sub", { title: pack.covers }, pack.covers),
      h("span.meta", null, isRerailPack(pack) ? packMeta(pack) : `${packMeta(pack)} · ${publisherLabel(pack)}`)),
    asking ? null : h("div.pend", null, h("span.pstatus", null, status), action),
    question);
}

const CATALOG_LINE = "Rule packs for languages, frameworks and code quality. Add one to this repo, or remove it later.";

function catalog() {
  const all = listed(), kinds = catalogKinds(all);
  if (!kinds.some(([k]) => k === kind)) kind = "all";
  const shown = catalogPacks(all, kind);
  const added = shown.filter(isAttached).length;
  return h("div.catalog", null,
    h("div.ghead", null, h("h2", null, "Rule packs", h("span.n2", null, `${fmt(added)} of ${fmt(shown.length)} added`)),
      h("p", null, CATALOG_LINE)),
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
  const kept = keptPacks().includes(pack.id), repo = repoName() || "this repo";
  return h("div.prow2", { "data-act": "viewpack", "data-i": pack.id, role: "button", tabindex: "0" },
    packIcon(pack),
    h("div.pt2", null, h("b", null, pack.name), h("span.sub", { title: pack.covers }, pack.covers),
      reason === null ? null : h("span.why", null, h("b", null, "Why"), reason),
      h("span.meta", null, packMeta(pack))),
    h("div.pact2", null, reason !== null || kept
      ? h("span.swl", null,
        h("button.sw", { role: "switch", "aria-checked": String(kept), "aria-label": `${pack.name} in ${repo}`, "data-act": "reviewtoggle", "data-i": pack.id }))
      : h("button.btn.sm", { "data-act": "reviewtoggle", "data-i": pack.id }, icon("plus"), "Add pack")));
}

/** What is different while Rerail is not started: nothing the switches keep runs yet. */
function reviewBanner(repo, picked) {
  if (stateHere() === "started") return null;
  const n = picked.length;
  return h("div.welcome", { role: "status" }, h("div", null,
    h("b", null, n === 0 ? `No rule pack picked for ${repo}.` : `${fmt(n)} ${plural(n, "rule pack")} picked for ${repo}.`),
    h("p", null, `Nothing is on yet. ${n === 0 ? "Add any that fit" : "Switch off any you don’t want"}, then start Rerail from your coding agent session.`)));
}

function review() {
  const repo = repoName() || "this repo";
  if (data.recommendations === null) return h("div.catalog.rv", null, h("div.list", null, h("div.empty", null, h("p", null, `Reading ${repo}…`))));
  const packs = catalogPacks(listed(), "all");
  const reasons = new Map(data.recommendations.map(r => [r.packId, r.reason ?? ""]));
  const picked = packs.filter(p => reasons.has(p.id)), others = packs.filter(p => !reasons.has(p.id));
  return h("div.catalog.rv", null,
    h("div.cathead", null, h("button.btn.sm.ghost.link", { "data-act": "nav", "data-i": "overview" }, icon("back"), repo)),
    reviewBanner(repo, picked),
    h("div.ghead", null, h("h2", null, "Rule packs"), h("p", null, CATALOG_LINE)),
    h("div.list", null, picked.map(pack => reviewRow(pack, reasons.get(pack.id)))),
    others.length ? h("div.ghead.top", null, h("h2", null, "Other rule packs", h("span.n2", null, `${fmt(others.length)} not picked`)),
      h("p", null, "Add any that fit.")) : null,
    others.length ? h("div.list", null, others.map(pack => reviewRow(pack, null))) : null);
}

export function render(root) {
  const view = ui.view;
  const sig = JSON.stringify([view, kind, data.packs.map(p => p.id), [...data.attachedPacks.keys()], isSetUp(), data.recommendations, ui.catView, ui.removeAsk,
    [...ui.removedPacks], repoName(), stateHere(), runningCount()]);
  setContent(root, sig, () => (view === "review" ? review() : catalog()));
}

/** The catalog opens on every kind; the review reads the repository for the packs set-up would pick, once per repository. */
export function enter() {
  if (ui.view === "packs") kind = "all";
  if (ui.view !== "review" || ui.scope === null || data.recommendations !== null) return;
  loadRecommendations().then(invalidate, error => toast(failed(`Couldn’t read ${repoName() || "this repo"} to pick rule packs`, error)));
}

export const actions = {
  catkind: k => { kind = k; invalidate(); },
  catview: v => { ui.catView = v; invalidate(); },
  reviewtoggle: id => keepPack(id, !keptPacks().includes(id)),
  // A pack removed from this page, wanted back: added again, as Add pack would.
  undoremove: id => { ui.removedPacks.delete(id); commonActions.addpack(id); },
};
