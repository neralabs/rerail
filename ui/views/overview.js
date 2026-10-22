/**
 * Overview (`#overview`, the default; `#rules` before v8): the banner a set-up repository may need (stopped, no rule
 * packs, day one), the range's tiles, and every rule this repository has, under the rule list's controls.
 */
import { h, setText, setClass, setContent, icon } from "../dom.js";
import { fmt, plural, startOfDay } from "../format.js";
import { data, ui, rangeLabel, repoName, stateHere, runningCount, summary } from "../state.js";
import { loadRecommendations } from "../poll.js";
import { invalidate, toast, writes } from "../shell.js";
import { rangeControl, syncRange } from "../render/range.js";
import { renderRules, ruleListActions } from "../render/rules.js";
import { ruleControls, ruleControlActions, ruleControlInput, ruleControlChange } from "../render/rule-controls.js";

export const needs = ["rules", "packs", "activity", "newestSession"];

/* ---------- banners ----------
 * At most one at a time, above the tiles of a set-up repository:
 * - stopped: what that means and how to start;
 * - W0, for a repo with no rule packs attached: ask Rerail to recommend, then add them in one go;
 * - the day-one set-up banner: how many rules are on, and what to do next.
 * A repository that is not set up shows the Not set up page instead (`not-set-up.js`). */

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
      h("p", null, "Every rule pack is still in Add rule packs, if one suits how your team works.")), dismiss("recommendno"));
  }
  // Each pack with what was found for it, as set-up's step 4 shows it.
  const why = recs.map(r => `${r.pack.name}: ${r.reason}.`).join(" ");
  return h("div.welcome", { role: "status" }, h("div", null,
    h("b", null, `Rerail read ${repo}: ${fmt(recs.length)} rule ${plural(recs.length, "pack")} ${recs.length === 1 ? "looks" : "look"} right`),
    h("p", null, `${why} Nothing changes until you say so.`),
    h("p.acts", null, h("button.btn.sm.solid", { "data-act": "addrecommended" }, icon("plus"), "Add these rule packs"), h("button.btn.sm.ghost.link", { "data-act": "recommendno" }, "Not now"))));
}

const welcomeKey = () => `rerail-welcome-dismissed:${data.repository?.repositoryKey ?? ""}`;

function dismissWelcome() {
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

function renderBanners(root) {
  const b = banner();
  setContent(root, JSON.stringify([b === null ? null : b.textContent, runningCount(), ui.recommend]), () => b);
}

/* ---------- tiles ---------- */

const tile = () => h("div.tile", null, h("div.k"), h("div.v.num", null, h("span.big"), h("small")), h("div.s"));

function setTile(el, k, big, small, s) {
  setText(el.querySelector(".k"), k);
  setText(el.querySelector(".big"), big);
  setText(el.querySelector("small"), small);
  setText(el.querySelector(".s"), s);
}

function steeredNote(sum, label) {
  if (sum.rerails === 0) return sum.rules.length === 0 ? "nothing to steer yet" : `none ${label}`;
  const logged = sum.logged ? ` · ${fmt(sum.logged)} logged only` : "";
  return `${label} · across ${sum.steerRules} ${plural(sum.steerRules, "rule")}${logged}`;
}

function sessionsNote(sum, label) {
  const sir = sum.sessionsInRange;
  if (sir) return `${label} · ${fmt(Math.max(0, sir - sum.sessionsWithTriggers))} had none`;
  return sum.anySessions ? `no sessions ${label}` : "your next coding agent session shows here";
}

function renderTiles(root, tiles, sum) {
  const [t1, t2, t3, t4] = tiles;
  const label = rangeLabel();
  setTile(t1, "Steered", fmt(sum.rerails), "", steeredNote(sum, label));
  const sir = sum.sessionsInRange;
  setTile(t2, "Sessions with violations logged", sir ? fmt(sum.sessionsWithTriggers) : "0", sir ? ` of ${fmt(sir)}` : "", sessionsNote(sum, label));
  setTile(t3, "Rules with violations logged", fmt(sum.firedN), ` of ${fmt(sum.on.length)}`,
    sum.firedN ? `${label} · ${fmt(sum.on.length - sum.firedOn)} on with none` : `none ${label}`);
  setTile(t4, "Rules on", fmt(sum.on.length), ` of ${fmt(sum.rules.length)}`,
    `${fmt(sum.logOn)} log · ${fmt(sum.steerOn)} steer · 0 block`);
  setClass(root, "is-empty", data.loaded && sum.rules.length === 0);
}

/* ---------- the view ---------- */

let page = null;

function build() {
  const banners = h("div.banners");
  const range = rangeControl();
  const tiles = [tile(), tile(), tile(), tile()];
  const rollup = h("div.rollup", null, h("div.rollhead", null, h("h2", null, "Overview"), range), h("div.tiles", null, tiles));
  const controls = h("div");
  const list = h("div");
  page = { root: h("div.ov", null, banners, rollup, controls, list), banners, range, tiles, rollup, controls, list };
}

export function render(root) {
  if (page === null) build();
  if (page.root.parentNode !== root) root.replaceChildren(page.root);
  const sum = summary();
  renderBanners(page.banners);
  syncRange(page.range);
  renderTiles(page.rollup, page.tiles, sum);
  const controls = ruleControls();
  if (controls.parentNode !== page.controls) page.controls.replaceChildren(controls);
  renderRules(page.list, sum);
}

export const actions = {
  ...ruleListActions,
  ...ruleControlActions,
  recommend: async () => {
    ui.recommend = "loading"; invalidate();
    try { await loadRecommendations(); ui.recommend = "shown"; } catch (error) { ui.recommend = "idle"; toast(`Could not read the repo: ${error.message}`); }
    invalidate();
  },
  recommendno: () => { ui.recommend = "dismissed"; invalidate(); },
  addrecommended: () => {
    const ids = (data.recommendations ?? []).map(r => r.packId);
    writes.addPacks(ids, `${ids.length} rule ${ids.length === 1 ? "pack" : "packs"} added. Their rules are on.`);
  },
  dismisswelcome: () => { dismissWelcome(); invalidate(); },
};

export const onInput = ruleControlInput;
export const onChange = ruleControlChange;
