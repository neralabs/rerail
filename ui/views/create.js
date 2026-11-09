/**
 * Create a rule (`#create`): rules are written with the coding agent, so the page shows the command to type, what
 * happens next and where the rule appears. "Places to start" lists the rule packs Rerail recommends for this
 * repository that are not added yet; with none, the section is left out.
 */
import { h, setContent, icon } from "../dom.js";
import { data, ui, repoName } from "../state.js";
import { failed } from "../format.js";
import { loadRecommendations } from "../poll.js";
import { invalidate, toast } from "../shell.js";
import { CREATE_COMMAND } from "../render/rules.js";

export const needs = [];

/** The recommended packs not added here, with the name and the reason to show: `[{ packId, name, reason }]`. */
export function createSuggestions(recommendations, attachedPacks, packs) {
  const seen = new Set();
  return (recommendations ?? []).flatMap(({ packId, reason }) => {
    const pack = packs.find(p => p.id === packId);
    if (!pack || attachedPacks.has(packId) || seen.has(packId)) return [];
    seen.add(packId);
    return [{ packId, name: pack.name, reason }];
  });
}

function commandBlock(command) {
  return h("div.cmd", null, h("code", null, command), h("button.btn.sm.solid", { "data-act": "cmdcopy", "data-i": command }, icon("copy"), "Copy"));
}

function placesToStart(suggestions) {
  if (suggestions.length === 0) return null;
  return h("section.psec.places", null, h("h3", null, "Places to start"),
    suggestions.map(s => h("div.place", null,
      h("p.pintro", null, `The ${s.name} rule pack ${s.reason}. It is not added here yet.`),
      h("div.acts", null,
        h("button.btn.sm", { "data-act": "addpack", "data-i": s.packId }, icon("plus"), `Add ${s.name}`),
        h("button.btn.sm.ghost.link", { "data-act": "nav", "data-i": `pack/${s.packId}` }, "See its rules")))));
}

export function render(root) {
  const suggestions = createSuggestions(data.recommendations, data.attachedPacks, data.packs);
  setContent(root, JSON.stringify(["create", suggestions]), () => h("div.cpage.pbody", null,
    h("h1", null, "Create a rule"),
    h("p.pintro", null, "Rules are written with your coding agent. Open a session in this repo and type:"),
    commandBlock(CREATE_COMMAND),
    h("p.pintro", null, "Your coding agent writes a rule from your words, suggests rules from this repo’s sessions and standards, or fixes one of yours that misfires. Each is tested on this repo and read back before it’s added."),
    h("p.pintro", null, "It lands under Created by you, switched on, and its violations show in History."),
    placesToStart(suggestions)));
}

/** Read the recommendations once per repository, for Places to start. */
export function enter() {
  if (ui.scope === null || data.recommendations !== null) return;
  loadRecommendations().then(invalidate, error => toast(failed(`Couldn’t read ${repoName() || "this repo"} to suggest rule packs`, error)));
}
