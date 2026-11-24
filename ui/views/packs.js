/**
 * Add rule packs (`#packs`; `#rule-packs` before v8): every rule pack installed on this machine, then, once the daemon
 * has read them from npm, every other published pack that fits this Rerail, each part alphabetical, filtered by kind,
 * as a list or as cards; once the recommendations are read, the packs Rerail suggests for this repository first, each
 * with why, then the others. Each installed pack has its version and where it stands (Added, or Removed with Undo
 * until the page is left) on a line under its name, and Add pack or a quiet Remove pack that asks first, its question
 * on a line of its own under the pack's text. A pack is added whole: its rules arrive as the pack starts them, opt-in
 * ones off. A pack only published has its version on npm and the command that installs it, with Copy; once it is
 * installed it reads as the others, with Add pack. The review
 * (`#packs?review=1`, set-up's `#rule-packs?review=1`): the packs set-up picked for this repository, each with why
 * and a switch, then the others with Add pack. The person starts Rerail back in the terminal or coding agent.
 */
import { h, setContent, icon } from "../dom.js";
import { failed, fmt, plural } from "../format.js";
import { isRerailPack, isRunning, isWorktreePack, publisherLabel } from "../derive.js";
import { data, ui, isAttached, repoName, stateHere, isSetUp, keptPacks, runningCount } from "../state.js";
import { captureScope, inScope, publishedPacksFor, recommendationsFor } from "../poll.js";
import { invalidate, toast } from "../shell.js";
import { commonActions, keepPack } from "../actions.js";
import { catalogEntries, catalogKinds, catalogPacks, catalogSuggestions, installCommand, suggestedReason, suggestedSplit } from "../pack-model.js";
import { packAction, packIcon, packKind, removePackControl, suggestedBox } from "../render/pack-panel.js";

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

/** The line under a pack's name: its version, then where it stands here (Added, Removed); `null` when there is neither. */
function statusLine(pack, status) {
  const version = pack.version ? h("span.badge.pver", { title: `${pack.name} ${pack.version}, on this computer` }, `v${pack.version}`) : null;
  return version || status ? h("span.pstate", null, version, status) : null;
}

/** "Why suggested", for a pack Rerail suggests for this repository with a reason. */
function whyBox(pack) {
  const reason = suggestedReason(data.recommendations, pack.id);
  return reason ? suggestedBox("Why suggested", reason) : null;
}

/**
 * A pack as a card of the catalog's card view: its name and kind, its status line, what it covers, why it is suggested,
 * and its counts and action.
 */
const catalogCard = (pack, status, action, question) => h("div.pcard", { "data-act": "viewpack", "data-i": pack.id, role: "button", tabindex: "0" },
  h("div.pc-top", null, packIcon(pack), h("div.pc-name", null, h("b", null, pack.name), h("span.kind", null, packKind(pack)))),
  statusLine(pack, status),
  h("span.sub", { title: pack.covers }, pack.covers),
  whyBox(pack),
  question,
  h("div.pc-foot", null, h("span.meta", null, packMeta(pack)), action));

/**
 * A pack published on npm that is not installed: its version there, what it covers, why it is suggested, its rule
 * count, and the command that installs it, with Copy. It opens no sheet: its rules are read once it is installed.
 */
function publishedItem(pack) {
  const command = installCommand(pack), n = pack.ruleCount;
  const status = h("span.pstate", null, h("span.badge.pver", { title: `${pack.name} ${pack.version}, on npm` }, `v${pack.version}`), h("span.notinst", null, "Not installed"));
  const why = pack.reason === undefined ? null : suggestedBox("Why suggested", pack.reason);
  const how = h("span.meta.how", null, "Install it with ", h("code", null, command), ", then add it here.");
  const copy = h("button.btn.sm", { "data-act": "cmdcopy", "data-i": command, title: `${command}, to run in a terminal` }, icon("copy"), "Copy command");
  const meta = `${fmt(n)} ${plural(n, "rule")}`;
  if (ui.catView === "cards") {
    return h("div.pcard.pub", null,
      h("div.pc-top", null, packIcon(pack), h("div.pc-name", null, h("b", null, pack.name), h("span.kind", null, packKind(pack)))),
      status, h("span.sub", { title: pack.covers }, pack.covers), why, how,
      h("div.pc-foot", null, h("span.meta", null, meta), copy));
  }
  return h("div.prow2.pub", null,
    packIcon(pack), h("div.pt2", null, h("b", null, pack.name), status, h("span.sub", { title: pack.covers }, pack.covers), why,
      h("span.meta", null, isRerailPack(pack) ? meta : `${meta} · ${publisherLabel(pack)}`), how),
    h("div.pend", null, copy));
}

function catalogItem(pack) {
  if (pack.published) return publishedItem(pack);
  const asking = ui.removeAsk === pack.id;
  const { status, action } = asking ? { status: null, action: null } : catalogState(pack);
  const question = asking ? h("div.rmline", null, removePackControl(pack)) : null;
  if (ui.catView === "cards") return catalogCard(pack, status, action, question);
  return h("div.prow2", { "data-act": "viewpack", "data-i": pack.id, role: "button", tabindex: "0" },
    packIcon(pack), h("div.pt2", null, h("b", null, pack.name), statusLine(pack, status), h("span.sub", { title: pack.covers }, pack.covers), whyBox(pack),
      h("span.meta", null, isRerailPack(pack) ? packMeta(pack) : `${packMeta(pack)} · ${publisherLabel(pack)}`)),
    asking ? null : h("div.pend", null, action),
    question);
}

const CATALOG_LINE = "Rule packs are ready-made rules, mostly the same ones linters use (ESLint, Ruff, Clippy…), checked while your coding agent writes. Add one to this repo, or remove it later.";

function catalog() {
  const all = catalogEntries(listed(), data.publishedPacks), kinds = catalogKinds(all);
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
    catalogBody(shown),
    publishedNote());
}

/** Under the packs, while the published ones are read, or why they could not be. */
function publishedNote() {
  const published = data.publishedPacks;
  if (published === null) return h("p.pubnote", { role: "status" }, "Reading the other rule packs published on npm…");
  return published.unread === null ? null : h("p.pubnote", { role: "status" }, failed("Couldn’t read the other rule packs published on npm", published.unread));
}

/** Packs as the view chosen shows them: cards or a list. */
const packBox = packs => (ui.catView === "cards" ? h("div.cards", null, packs.map(catalogItem)) : h("div.list", null, packs.map(catalogItem)));

/** A group's heading in the catalog: its title, its count, and one line. */
const groupHead = (title, n, line) => h("div.ghead.grp", null, h("h3", null, title, h("span.n2", null, fmt(n))), h("p", null, line));

/**
 * The catalog's packs: one list until the recommendations or the published packs are read and suggest a pack; then the
 * packs suggested for this repository, then the others.
 */
function catalogBody(shown) {
  const split = suggestedSplit(shown, catalogSuggestions(data.recommendations, shown));
  if (split === null) return packBox(shown);
  const repo = repoName() || "this repo", { suggested, others } = split;
  return h("div.groups", null,
    suggested.length ? [groupHead(`Suggested for ${repo}`, suggested.length, `Rerail picked these from what ${repo} is built with.`), packBox(suggested)] : null,
    others.length ? [othersHead(repo, others.length, suggested.length > 0), packBox(others)] : null);
}

/** The other packs' heading; when the kind chosen has no suggested pack, its packs are simply the rule packs. */
const othersHead = (repo, n, afterSuggested) => (afterSuggested
  ? groupHead("Other rule packs", n, `Not matched to ${repo}. Add any you want.`)
  : groupHead("Rule packs", n, "None of these is suggested for this kind of pack."));

/* ---------- the review ---------- */

function reviewRow(pack, reason) {
  const kept = keptPacks().includes(pack.id), repo = repoName() || "this repo";
  return h("div.prow2", { "data-act": "viewpack", "data-i": pack.id, role: "button", tabindex: "0" },
    packIcon(pack),
    h("div.pt2", null, h("b", null, pack.name), h("span.sub", { title: pack.covers }, pack.covers),
      reason === null ? null : h("span.why", null, h("b", null, "Why suggested"), reason),
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
    h("b", null, n === 0 ? `No rule pack matches what ${repo} is built with.` : `${fmt(n)} ${plural(n, "rule pack")} suggested for what ${repo} is built with.`),
    h("p", null, `Nothing is on yet. ${n === 0 ? "Add any you want" : "Switch off any you don’t want"}, then start Rerail from your coding agent session.`)));
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
    others.length ? h("div.ghead.top", null, h("h2", null, "Other rule packs", h("span.n2", null, `${fmt(others.length)} not suggested`)),
      h("p", null, "Add any you want.")) : null,
    others.length ? h("div.list", null, others.map(pack => reviewRow(pack, null))) : null);
}

export function render(root) {
  const view = ui.view;
  const sig = JSON.stringify([view, kind, data.packs.map(p => [p.id, p.version]), [...data.attachedPacks.keys()], isSetUp(), data.recommendations, data.publishedPacks, ui.catView, ui.removeAsk,
    [...ui.removedPacks], repoName(), stateHere(), runningCount()]);
  setContent(root, sig, () => (view === "review" ? review() : catalog()));
}

/**
 * The catalog opens on every kind; both it and the review read the repository for the packs Rerail suggests, once per
 * repository, and the catalog the packs published on npm, which the daemon reads when they are first asked for.
 */
export function enter() {
  if (ui.view === "packs") kind = "all";
  if (ui.scope === null) return;
  const scope = captureScope(), repo = repoName() || "this repo", view = ui.view;
  if (view === "packs" && data.publishedPacks === null) publishedPacksFor(scope).then(invalidate);
  if (data.recommendations !== null) return;
  recommendationsFor(scope).then(invalidate, error => readFailed(scope, view, `Couldn’t read ${repo} to pick rule packs`, error));
}

/**
 * A failed read of the recommendations says so on the review, which cannot list its picks without them, while the person
 * is still on it for that repository. The catalog says nothing: it stays one list, and the next visit reads again.
 */
function readFailed(scope, view, title, error) {
  if (view === "review" && ui.view === "review" && inScope(scope)) toast(failed(title, error));
}

export const actions = {
  catkind: k => { kind = k; invalidate(); },
  catview: v => { ui.catView = v; invalidate(); },
  reviewtoggle: id => keepPack(id, !keptPacks().includes(id)),
  // A pack removed from this page, wanted back: added again, as Add pack would.
  undoremove: id => { ui.removedPacks.delete(id); commonActions.addpack(id); },
};
