/**
 * The sidebar (`#sidenav`): Overview; one item per rule pack added here with how many of its rules are on, and
 * Created by you once there is a rule created by you; Add rule packs and Create a rule; History by rule and by
 * session; Merge Assist, with how many of its findings wait for the person; Settings. Each item is a `nav` action to
 * its view's hash (`routes.js`).
 */
import { h, setContent, icon } from "../dom.js";
import { fmt, plural } from "../format.js";
import { isRunning } from "../derive.js";
import { data, ui, ruleSections, repoName } from "../state.js";
import { packIcon } from "../render/pack-panel.js";
import { waitingCount } from "../merge-model.js";

/** A section's rules that run, of all of them: "24/71" in the item, "24 of 71 rules on" as its tooltip and accessible name. */
function onCount(section) {
  const all = section.groups.flatMap(g => g.all);
  const on = fmt(all.filter(isRunning).length), total = fmt(all.length);
  return { short: `${on}/${total}`, long: `${on} of ${total} rules on` };
}

/** Where a pack sorts, as v8 ranks it: the packs set-up recommended here in their order, then the rest by name. */
function packRank(pack) {
  const at = (data.recommendations ?? []).findIndex(r => r.packId === pack.id);
  return at < 0 ? Number.MAX_SAFE_INTEGER : at;
}

/** The items the repository adds: Created by you first (your own rule pack, beside Rerail's), then its packs. */
function repositoryItems() {
  const sections = ruleSections();
  const mine = sections.find(s => s.kind === "mine");
  const packs = sections.filter(s => s.kind === "pack")
    .sort((a, b) => packRank(a.pack) - packRank(b.pack) || a.pack.name.localeCompare(b.pack.name, undefined, { sensitivity: "base" }));
  return [
    ...(mine !== undefined && mine.groups[0].all.length > 0
      ? [{ hash: "created", label: "Created by you", n: onCount(mine), mine: true, current: ui.view === "created" }] : []),
    ...packs.map(s => ({ hash: `pack/${s.pack.id}`, label: s.pack.name, n: onCount(s), pack: s.pack, current: ui.view === "pack" && ui.pack === s.pack.id })),
  ];
}

/**
 * Merge Assist's findings that wait for the person (decided differently, needs review): "3", or nothing at 0 and while
 * the last read failed (the count may no longer hold).
 */
function mergeCount() {
  const n = data.mergeAssistError === null ? waitingCount(data.mergeAssist, ui.mergePending) : 0;
  if (n === 0) return null;
  const long = `${fmt(n)} ${plural(n, "finding waits", "findings wait")} for you`;
  return { short: fmt(n), long, cls: ".due" };
}

/** A sidebar item; a long pack name wraps to a second line, its icon and count staying on the first. */
function item(hash, label, glyph, { current = ui.view === hash, n = null, cls = "", title = label } = {}) {
  return h(`button.sn-item${cls}`, { "data-act": "nav", "data-i": hash, "aria-current": current ? "page" : "false", title },
    glyph, h("span.t", null, label), n === null ? null : h(`span.n${n.cls ?? ""}`, { title: n.long, "aria-label": n.long }, n.short));
}

export function renderSidenav(root) {
  const scoped = ui.scope !== null && !data.missing && data.repository !== null;
  const setUp = scoped && data.repository.setUp;
  const items = setUp ? repositoryItems() : [];
  const note = !scoped ? null
    : !setUp ? `${repoName()} is not set up.`
      : items.length === 0 && data.loaded ? "No rule packs yet." : null;
  const merge = scoped ? mergeCount() : null;
  const signature = JSON.stringify([ui.view, ui.pack, setUp, scoped, note, merge?.short, items.map(i => [i.hash, i.label, i.n?.short, i.current])]);
  setContent(root, signature, () => [
    item("overview", "Overview", icon("overview", 14)),
    h("div.sn-h", null, "Rule packs"),
    items.map(i => item(i.hash, i.label, i.mine ? icon("pencil", 14) : packIcon(i.pack), { current: i.current, n: i.n })),
    note === null ? null : h("div.sn-empty", null, note),
    item("packs", "Add rule packs", icon("plus", 14), { cls: ".add" }),
    setUp ? item("create", "Create a rule", icon("plus", 14), { cls: ".add", title: "Rules are written with your coding agent. Shows the command to type." }) : null,
    h("div.sn-h", null, "History"),
    item("history", "By rule", icon("rule", 14)),
    item("sessions", "By session", icon("session", 14)),
    scoped ? h("div.sn-h", null, "Experimental") : null,
    scoped ? item("merge", "Merge Assist", icon("repo", 14), { n: merge, title: "Share the decisions you make with your coding agent between branches" }) : null,
    h("div.sn-foot", null, item("settings", "Settings", icon("gear", 14))),
  ]);
}
