/**
 * Banners above the Rules overview of a set-up repository, at most one at a time:
 * - stopped: what that means and how to start;
 * - W0, for a repo with no rule packs attached: ask Rerail to recommend, then add them in one go;
 * - the day-one set-up banner: how many rules are on, and what to do next.
 * A repository that is not set up shows its own "Not set up" page instead (app.js).
 */
import { h, setContent, icon } from "../dom.js";
import { fmt, plural, startOfDay } from "../format.js";
import { data, ui, repoName, stateHere, runningCount } from "../state.js";

const dismiss = act => h("button.x", { "data-act": act, "aria-label": "Dismiss" }, "✕");

function stoppedBanner(repo) {
  return h("div.welcome.warn", { role: "status" }, h("div", null,
    h("b", null, `Rerail is stopped in ${repo}.`),
    h("p", null, "Nothing is audited or logged here; your rules and history are kept. The button starts it in every checkout of the repo; ",
      h("code", null, "/rerail-start"), " in a session starts it in that worktree.")),
  h("button.btn.sm.solid", { "data-act": "repostart", "data-i": data.repository.repositoryKey }, "Start Rerail"));
}

function recommendBanner(repo) {
  if (ui.recommend === "loading") return h("div.welcome", { role: "status" }, h("div", null, h("b", null, `Reading ${repo}…`)));
  if (ui.recommend !== "shown" || data.recommendations === null) {
    return h("div.welcome", { role: "status" }, h("div", null,
      h("b", null, `No rule packs are on in ${repo}.`),
      h("p", null, `Changed your mind? Rerail can read ${repo} on this computer and recommend the rule packs that look right. Nothing changes until you say so.`),
      h("p.acts", null, h("button.btn.sm.solid", { "data-act": "recommend" }, "Recommend rule packs"), h("button.btn.sm.ghost.link", { "data-act": "recommendno" }, "Not now"))));
  }
  const recs = data.recommendations.map(r => ({ ...r, pack: data.packs.find(p => p.id === r.packId) })).filter(r => r.pack);
  if (recs.length === 0) {
    return h("div.welcome", { role: "status" }, h("div", null,
      h("b", null, `Rerail read ${repo} and found no rule packs that fit.`),
      h("p", null, "Every rule pack is still in Rule packs, if one suits how your team works.")), dismiss("recommendno"));
  }
  // Each pack with what was found for it, as set-up's step 4 shows it.
  const why = recs.map(r => `${r.pack.name}: ${r.reason}.`).join(" ");
  return h("div.welcome", { role: "status" }, h("div", null,
    h("b", null, `Rerail read ${repo}: ${fmt(recs.length)} rule ${plural(recs.length, "pack")} ${recs.length === 1 ? "looks" : "look"} right`),
    h("p", null, `${why} Nothing changes until you say so.`),
    h("p.acts", null, h("button.btn.sm.solid", { "data-act": "addrecommended" }, icon("plus"), "Add these rule packs"), h("button.btn.sm.ghost.link", { "data-act": "recommendno" }, "Not now"))));
}

const welcomeKey = () => `rerail-welcome-dismissed:${data.repository?.repositoryKey ?? ""}`;
export function dismissWelcome() {
  ui.welcomeDismissed = true;
  try { localStorage.setItem(welcomeKey(), "1"); } catch { /* private mode: dismissed for this visit */ }
}

/** Day one: set up today, and no session has started since. */
function showWelcome(repository) {
  if (ui.welcomeDismissed) return false;
  try { if (localStorage.getItem(welcomeKey()) === "1") return false; } catch { /* storage blocked */ }
  const at = repository.configuredAt;
  return at !== null && at >= startOfDay() && !(data.newestSession !== null && data.newestSession.startedAt >= at);
}

function welcomeBanner(repo) {
  const on = runningCount();
  return h("div.welcome", { role: "status" }, icon("done", 22), h("div", null,
    h("b", null, `Rerail is set up. ${fmt(on)} ${plural(on, "rule")} ${on === 1 ? "is" : "are"} on.`),
    h("p", null, `Open a new coding agent session in ${repo} and start coding. Sessions that were already open won’t have Rerail; nothing needs restarting.`)),
  dismiss("dismisswelcome"));
}

function banner() {
  const repository = data.repository;
  if (!data.loaded || repository === null || !repository.setUp) return null;
  const repo = repoName();
  if (stateHere() === "stopped") return stoppedBanner(repo);
  if (data.attachedPacks.size === 0 && ui.recommend !== "dismissed") return recommendBanner(repo);
  return showWelcome(repository) ? welcomeBanner(repo) : null;
}

export function renderBanners(root) {
  const b = banner();
  const on = runningCount();
  setContent(root, JSON.stringify([b === null ? null : b.textContent, on, ui.recommend]), () => b);
}
