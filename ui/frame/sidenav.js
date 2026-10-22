/**
 * The sidebar (`#sidenav`): Overview; one item per rule pack added here with how many of its rules are on, and
 * Created by you once there is a rule created by you; Add rule packs and Create a rule; History by rule and by
 * session; Settings. Each item is a `nav` action to its view's hash (`routes.js`).
 */
import { h, setContent, icon } from "../dom.js";
import { fmt } from "../format.js";
import { isRunning } from "../derive.js";
import { data, ui, ruleSections, repoName } from "../state.js";
import { packIcon } from "../render/pack-panel.js";

/** "3 of 8 on": a section's rules that run, of all of them. */
function onCount(section) {
  const all = section.groups.flatMap(g => g.all);
  return `${fmt(all.filter(isRunning).length)} of ${fmt(all.length)} on`;
}

/** The items the repository adds: its packs, then Created by you. */
function repositoryItems() {
  const items = [];
  for (const section of ruleSections()) {
    if (section.kind === "pack") {
      items.push({ hash: `pack/${section.pack.id}`, label: section.pack.name, n: onCount(section), pack: section.pack,
        current: ui.view === "pack" && ui.pack === section.pack.id });
    } else if (section.kind === "mine" && section.groups[0].all.length > 0) {
      items.push({ hash: "created", label: "Created by you", n: onCount(section), mine: true, current: ui.view === "created" });
    }
  }
  // Created by you comes after the packs.
  return [...items.filter(i => !i.mine), ...items.filter(i => i.mine)];
}

function item(hash, label, glyph, { current = ui.view === hash, n = null, cls = "" } = {}) {
  return h(`button.sn-item${cls}`, { "data-act": "nav", "data-i": hash, "aria-current": current ? "page" : "false" },
    glyph, h("span.t", null, label), n === null ? null : h("span.n", null, n));
}

export function renderSidenav(root) {
  const scoped = ui.scope !== null && !data.missing && data.repository !== null;
  const setUp = scoped && data.repository.setUp;
  const items = setUp ? repositoryItems() : [];
  const note = !scoped ? null
    : !setUp ? `${repoName()} is not set up.`
      : items.length === 0 && data.loaded ? "No rules added yet. Add a rule pack or create a rule." : null;
  const signature = JSON.stringify([ui.view, ui.pack, setUp, note, items.map(i => [i.hash, i.label, i.n, i.current])]);
  setContent(root, signature, () => [
    item("overview", "Overview", icon("overview", 14)),
    h("div.sn-h", null, "Rule packs"),
    items.map(i => item(i.hash, i.label, i.mine ? icon("pencil", 14) : packIcon(i.pack), { current: i.current, n: i.n })),
    note === null ? null : h("div.sn-empty", null, note),
    item("packs", "Add rule packs", icon("plus", 14), { current: ui.view === "packs" || ui.view === "review", cls: ".add" }),
    setUp ? item("create", "Create a rule", icon("plus", 14), { cls: ".add" }) : null,
    h("div.sn-h", null, "History"),
    item("history", "By rule", icon("rule", 14)),
    item("sessions", "By session", icon("session", 14)),
    h("div.sn-foot", null, item("settings", "Settings", icon("gear", 14))),
  ]);
}
