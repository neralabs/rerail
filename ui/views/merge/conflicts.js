/**
 * Merge Assist's Conflicts, drawn into the page `merge.js` builds (`page.conflicts`): open conflicts as cards, the ones
 * not quiet first (each with its short id, its summary, a failed conflict hook, both sides, the evidence, and who to
 * talk it over with), quiet ones muted, then the dismissed ones folded with who dismissed each and when. A card's one
 * action is Dismiss (not a conflict), which `merge.js` sends. A card or row is keyed by its conflict's id and what it
 * shows, so a change rebuilds it and its age is patched in place; `showConflict` finds a linked one by that key.
 *
 * Conflict titles, summaries, hunks, authors, branch names and links are other people's text: every one is a text
 * node (`h()` children), never markup, and a pull request link is an `href` only when `safePeerUrl` accepts it.
 */
import { h, icon, setAttr, setContent, setText, syncChildren } from "../../dom.js";
import { ago, fmt, stamp } from "../../format.js";
import { data, ui } from "../../data/state.js";
import {
  QUIET_TEXT, TOLD_TEXT,
  conflictSections, hookFailure, hunkLines, quoted, safePeerUrl, scannedWhat, sideHeading,
} from "../../model/merge-model.js";

/** A time a row shows ("4m ago"); patched on every render, so it moves on without rebuilding the row. */
const ageSlot = () => h("span.mago");

/**
 * A side's pull request as a link that opens it on GitHub, or `null` when `safePeerUrl` does not accept its URL for
 * this repository.
 */
function peerLink(side) {
  const status = data.mergeAssist;
  const href = status === null ? null : safePeerUrl(side.url, { repositoryKey: status.repositoryKey, repository: status.readiness.github.repository });
  return href === null ? null : h("a.mlink", { href, target: "_blank", rel: "noopener noreferrer", title: `Open ${side.label} on GitHub` }, side.label);
}

/* ---------- conflict cards ---------- */

/** A side's change as a diff: added lines green, removed red, headers muted. */
const hunkBlock = hunk => h("pre.mhunk", null, hunkLines(hunk).map(line => h(`span.${line.kind}`, null, line.text, "\n")));

/** Where a side's change is and when it was pushed. */
function sideWhere(side) {
  if (side.location === null && side.tip === null) return null;
  return h("div.mloc", null,
    side.location === null ? null : h("code", null, side.location),
    side.tip === null ? null : h("span", null, `${side.tip.pushed ? "pushed" : "committed"} ${stamp(Date.parse(side.tip.at))}`));
}

/** One side of a conflict: who (a link when its pull request is safe to open), what, the hunk and where. */
function sideBox(side, which) {
  const link = peerLink(side);
  const who = link === null ? sideHeading(side) : [link, side.author ? ` · ${side.author}` : null];
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

/** Who to settle it with, and their branch or pull request (a link when it is safe to open). */
function talkTo(theirs) {
  const where = peerLink(theirs) ?? h("span.mono", null, theirs.label);
  return h("p.mtalk", null, "Talk it over with ", theirs.author ? h("b.mauthor", null, theirs.author) : "the author", " (", where,
    "). Rerail never picks a side; dismiss it if it isn’t a conflict.");
}

/** What the other side is doing, and a failed hook. */
function conflictNotes(conflict) {
  const failure = hookFailure(conflict);
  return [
    h("p.msummary", null, conflict.summary),
    failure === null ? null : h("p.mhookfail", null, icon("logged", 14), failure),
  ];
}

function conflictCard(conflict) {
  return h(conflict.quiet ? "article.mcard.quiet" : "article.mcard", null,
    h("div.mchead", null,
      h("h3.mctitle", null, conflict.title),
      h("span.mshort", { title: `Name it in chat as #${conflict.shortId}` }, `#${conflict.shortId}`),
      h("span.mcage", null, "Open for ", ageSlot(), conflict.quiet ? null : [" · ", TOLD_TEXT])),
    conflictNotes(conflict),
    h("div.mvs", null, sideBox(conflict.mine, "mine"), sideBox(conflict.theirs, "theirs")),
    whyConflict(conflict.evidence),
    talkTo(conflict.theirs),
    h("div.macts", null,
      h("button.btn.sm.ghost", {
        "data-act": "madismiss", "data-i": conflict.id,
        "aria-label": `Dismiss ${quoted(conflict.title)}: not a conflict`, title: "Not a conflict: Rerail stops raising it",
      }, "Dismiss"),
      conflict.quiet ? h("span.mquiet", null, icon("logged"), QUIET_TEXT) : null));
}

/**
 * What a card shows, for its key: a change rebuilds the card. Its age and Dismiss's state are patched. The repository
 * `gh` names is part of it, since its pull request links depend on it (`peerLink`).
 */
const conflictContent = c => [c.quiet, c.shortId, c.title, c.mine, c.theirs, c.evidence, c.summary, c.hook, data.mergeAssist?.readiness.github.repository];

function syncCards(list, items) {
  syncChildren(list, items, c => `${c.id}\u0000${JSON.stringify(conflictContent(c))}`, conflictCard, (node, conflict) => {
    setAttr(node.querySelector('button[data-act="madismiss"]'), "disabled", ui.mergeSwitching);
    setText(node.querySelector(".mago"), ago(Date.parse(conflict.detectedAt)).replace(/ ago$/u, ""));
  });
}

/** A dismissed conflict, folded: who dismissed it and when, and whose work it was with. */
function dismissedRow(conflict) {
  const by = conflict.dismissal;
  return h("article.mrow.mclosed", null,
    h("div.mchead", null, h("span.badge", null, "Not a conflict"), h("h3.mctitle", null, conflict.title)),
    h("div.mmeta", null,
      by === null ? h("span", null, "Dismissed") : [h("span", null, "Dismissed by ", h("b.mauthor", null, by.name)), ageSlot()],
      h("span", null, `with ${sideHeading(conflict.theirs)}`)));
}

const dismissedContent = c => [c.title, c.theirs.label, c.theirs.author, c.dismissal];

/** The dismissed rows, rebuilt only when what one shows changes; each one's age is patched in place. */
function syncDismissed(list, items) {
  syncChildren(list, items, c => `${c.id}\u0000${JSON.stringify(dismissedContent(c))}`, dismissedRow, (node, conflict) => {
    const age = node.querySelector(".mago");
    if (age !== null && conflict.dismissal !== null) setText(age, ago(Date.parse(conflict.dismissal.at)));
  });
}

/** A folded list whose summary opens it (`mafold` with `name`), kept open across renders in `ui.mergeOpen`. */
function foldedList(name) {
  const list = h("div.list.mlist"), summary = h("summary", { "data-act": "mafold", "data-i": name });
  return { root: h("details.mfold", null, summary, h("div.in", null, list)), summary, list };
}

export function buildConflicts() {
  const head = h("div.ghead"), list = h("div.mcards"), empty = h("div"), dismissed = foldedList("dismissed");
  return { root: h("section.msec.mconf", { "data-section": "conflicts" }, head, list, empty, dismissed.root), head, list, empty, dismissed };
}

const emptyBlock = empty => h("div.list", null, h("div.empty", null, h("h3", null, empty.title), empty.lines.map(line => h("p", null, line))));

function conflictsEmpty(status, dismissed) {
  const scan = status.lastScan;
  const lines = [scan === null
    ? "Rerail looks for conflicts on every scan and after your coding agent’s edits: work in open pull requests and in your other worktrees that overlaps yours."
    : `Rerail compared this branch with ${scannedWhat(scan)} ${ago(Date.parse(scan.at))} and found nothing that collides with it.`];
  lines.push("When work overlaps, it shows here with whose it is, so you can talk it over with them.");
  if (dismissed > 0) lines.push("Earlier ones are under Dismissed.");
  return { title: "No open conflicts", lines };
}

const CONFLICTS_LEAD = "Work in flight that overlaps yours. Rerail never picks a side: talk it over with the other side’s author, and dismiss one that isn’t a conflict.";

export function renderConflicts(page, status) {
  const c = page.conflicts, on = status !== null && status.enabled;
  setAttr(c.root, "hidden", !on);
  if (!on) return;
  const { open, dismissed } = conflictSections(status, ui.mergeDismissing);
  setContent(c.head, JSON.stringify(open.length), () => [
    h("h2", { tabindex: "-1" }, "Open conflicts", h("span.n2", { "aria-label": `${fmt(open.length)} open` }, fmt(open.length))),
    h("p", null, CONFLICTS_LEAD),
  ]);
  setAttr(c.list, "hidden", open.length === 0);
  syncCards(c.list, open);
  const empty = open.length === 0 ? conflictsEmpty(status, dismissed.length) : null;
  setContent(c.empty, JSON.stringify(empty), () => (empty === null ? null : emptyBlock(empty)));
  setAttr(c.dismissed.root, "hidden", dismissed.length === 0);
  setAttr(c.dismissed.root, "open", ui.mergeOpen.has("dismissed"));
  setText(c.dismissed.summary, `Dismissed (${fmt(dismissed.length)})`);
  syncDismissed(c.dismissed.list, dismissed);
}

/** Scroll the linked conflict's card (or folded row) into view and focus it, so a screen reader starts there. */
export function showConflict(page, conflict) {
  if (conflict === null) return;
  const list = conflict.status === "open" ? page.conflicts.list : page.conflicts.dismissed.list;
  const node = [...list.children].find(n => n.dataset.key?.startsWith(`${conflict.id}\u0000`));
  if (node === undefined) return;
  setAttr(node, "tabindex", "-1");
  node.scrollIntoView?.({ block: "start", behavior: "smooth" });
  node.focus({ preventScroll: true });
}
