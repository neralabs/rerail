/**
 * One rule pack in the side panel: what it covers, Add or Remove pack, once added how its rules treat linter
 * suppression comments with a switch for the whole pack, and its groups with their rules.
 */
import { h, setContent, chevron, icon } from "../dom.js";
import { fmt, plural } from "../format.js";
import { categoryLabel, inPack, packSuppressions } from "../derive.js";
import { data, ui, isAttached, packGroups, keptPacks, reviewing } from "../state.js";

/** Whether the pack is on where its button shows: kept, during the review of a repository not set up yet; else added. */
const packOn = pack => (reviewing() ? keptPacks().includes(pack.id) : isAttached(pack));

/** Add pack; or Remove pack, which asks once in place ("Remove all 74 rules in TSLint?") before it acts. */
export function packAction(pack, solid = false) {
  if (!packOn(pack)) return h(`button.btn.sm${solid ? ".solid" : ""}`, { "data-act": "addpack", "data-i": pack.id }, icon("plus"), "Add pack");
  if (ui.removeAsk !== pack.id) return h("button.btn.sm.danger", { "data-act": "removepack", "data-i": pack.id }, "Remove pack");
  const n = pack.rules.length;
  return h("span.rmask", { role: "alertdialog", "aria-label": `Remove ${pack.name}?` },
    h("span", null, `Remove ${n === 1 ? "the 1 rule" : `all ${fmt(n)} rules`} in ${pack.name}?`),
    h("button.btn.sm.danger", { "data-act": "removepack", "data-i": pack.id }, "Remove"),
    h("button.btn.sm.ghost.link", { "data-act": "cancelremove" }, "Cancel"));
}

/** The pack's category as people read it, e.g. "Language" or "Repository". */
export const packKind = pack => categoryLabel(pack.category);

/** Two letters for a pack's badge, from its name: the first letters of two words, else its first two. */
export function monogram(pack) {
  const words = pack.name.split(/[\s._-]+/).filter(Boolean);
  if (words.length > 1) return (words[0][0] + words[1][0]).toUpperCase();
  return pack.name.slice(0, 2).toUpperCase();
}

const MODE_NAME = { log: "Log", steer: "Steer", block: "Block" };
/** How a pack rule starts: its mode, and "off" for a rule the pack adds switched off. */
const startsAs = rule => `${MODE_NAME[rule.mode] ?? rule.mode}${rule.on ? "" : " · off"}`;

/** How these packs' rules start, from what each pack says: Steer, unless some start in Log. */
export function startModeText(packs) {
  const logged = packs.reduce((n, pack) => n + pack.rules.filter(r => r.mode === "log").length, 0);
  const steer = "your coding agent is told as it works, and fixes it.";
  if (logged === 0) return `Every rule starts in Steer: ${steer}`;
  return `Rules start in Steer: ${steer} ${logged === 1 ? "1 starts" : `${fmt(logged)} start`} in Log, recorded only.`;
}

export const packIcon = pack => h("span.picon", { "aria-hidden": "true" }, h(`span.ltr${monogram(pack).length > 2 ? ".l3" : ""}`, null, monogram(pack)));

/** Once the pack is added: whether its rules here honour suppression comments, and the pack's switch for them. */
function suppressionsLine(pack, suppressions) {
  if (suppressions === null) return null;
  return h("p.pintro", null, `Linter suppression comments, such as eslint-disable or noqa: ${suppressions.text}. `,
    h("button.btn.sm.ghost.link", { "data-act": "packsuppress", "data-i": pack.id, "data-v": suppressions.next },
      suppressions.next === "ignore" ? "Ignore them" : "Honour them"));
}

export function renderPackPanel(root, pack) {
  const added = isAttached(pack), groups = packGroups(pack);
  // Every rule the pack added in some folder: a rule may also be added on its own elsewhere, and that folder comes first.
  const present = [...data.rules.values()].filter(r => inPack(r, pack.id));
  // The pack's switch is written as the settings owner the page shows: the repository, or this worktree.
  const owner = ui.scope?.checkout === undefined ? "repository" : ui.owner;
  const suppressions = added && !reviewing() && present.length > 0 ? packSuppressions(present, pack.id, owner, data.checkout?.checkoutPath) : null;
  const sig = `pack|${pack.id}|${added}|${packOn(pack)}|${ui.removeAsk === pack.id}|${[...data.rules.keys()].length}|${JSON.stringify(suppressions)}`;
  setContent(root, sig, () => h("div.pwrap", null,
    h("div.phead", null,
      h("div.ptop", null, packIcon(pack), h("div", null, h("div.eyebrow", null, `Rule pack · ${packKind(pack)}`), h("h2", null, pack.name)),
        h("button.x", { "data-act": "close", "aria-label": "Close" }, "✕")),
      h("div.pact", null, h("span.muted", null, `${fmt(groups.length)} ${plural(groups.length, "group")} · ${fmt(pack.rules.length)} ${plural(pack.rules.length, "rule")}${added ? " · added" : ""}`),
        h("span.grow"), packAction(pack, true))),
    h("div.pbody", null,
      h("p.pintro", null, pack.covers, " ", added
        ? `Its rules are on its page in the menu; turn them on or off there.`
        : "Add it and its rules get a page of their own in the menu, each on or off and in Log or Steer as listed below. Nothing else changes."),
      suppressionsLine(pack, suppressions),
      groups.map(group => {
        const rules = pack.rules.filter(r => r.group === group);
        return h("details.pgrp", { open: true },
          h("summary", null, h("span.caret", { "aria-hidden": "true" }, chevron()), h("b", null, group), h("span.n", null, fmt(rules.length))),
          rules.map(rule => h("div.prow", null,
            data.rules.has(rule.id)
              ? h("button.t", { "data-act": "packrule", "data-i": rule.id }, rule.title)
              : h("span.t", null, rule.title),
            h("span.m", null, startsAs(rule)),
            h("span.mono", null, rule.id))));
      }))));
}
