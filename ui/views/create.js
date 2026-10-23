/**
 * Create a rule (`#create`): rules are written with the coding agent, so the page shows the command to type, what
 * happens next and where the rule appears. "Places to start" lists the rule packs Rerail recommends for this
 * repository that are not added yet; with none, the section is left out.
 */
import { h, setContent, icon } from "../dom.js";
import { data, ui, repoName } from "../state.js";
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
  const repo = repoName() || "this repo";
  const suggestions = createSuggestions(data.recommendations, data.attachedPacks, data.packs);
  setContent(root, JSON.stringify(["create", repo, suggestions]), () => h("div.cpage.pbody", null,
    h("h1", null, "Create a rule"),
    h("p.pintro", null, "Rules are written with your coding agent, in your own words. Open a session in ", h("span.mono", null, repo), ", a new one or one already running, and type:"),
    commandBlock(CREATE_COMMAND),
    h("p.pintro", null, "Your coding agent asks a few questions: what the rule should look for, where in the repo it applies, and what should happen when it finds a violation, log it or steer the agent. Then it writes the rule and tells you its id."),
    h("p.pintro", null, "The new rule appears under Created by you, on by default, and its violations show in History."),
    placesToStart(suggestions)));
}

/** Read the recommendations once per repository, for Places to start. */
export function enter() {
  if (ui.scope === null || data.recommendations !== null) return;
  loadRecommendations().then(invalidate, error => toast(`Could not read the repo: ${error.message}`));
}
