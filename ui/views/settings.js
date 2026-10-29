/**
 * Settings (`#settings`). General: theme, the status line, when Rerail was installed, where history is kept (and
 * clearing the shown repo's), and the shown repo's Follow .gitignore. Repos: every repo Rerail knows, with how many
 * rules it has and a switch where it has any. Coding agents: which are connected, with the command that changes it.
 * Uninstall: the command. Settings are the machine's: the page shows with or without a repository in scope.
 */
import { h, setContent, icon } from "../dom.js";
import { andList, failed, fmt, plural } from "../format.js";
import { send } from "../api.js";
import { data, ui, repoName } from "../state.js";
import { captureScope, changeScope, inScope, loadAgents, loadPresence, loadFollowGitignore, saveFollowGitignore } from "../poll.js";
import { applyTheme, invalidate, toast } from "../shell.js";
import { loadRepoRuleCounts, repoTag } from "../repo-rules.js";
import { repoCounts } from "../render/repos.js";
import { agentStatus, installedLabel } from "../settings-model.js";

export const needs = [];

const UNINSTALL_COMMAND = "rerail install --remove";

/** Clear history: asked (the row shows the question), then clearing (the request is out). */
let clearAsk = false;
let clearing = false;
/** The poll (`data.updatedAt`) the repos' rule counts were last read after. */
let countsRound = -1;

function row(label, sub, right, attrs = null) {
  return h("div.srow2", attrs, h("div.rt", null, label, sub ? h("span.sub", null, sub) : null), right);
}

function seg(act, options, current) {
  return h("div.rangectl", { role: "group" }, options.map(([v, l]) => h("button", { "data-act": act, "data-i": v, "aria-pressed": String(current === v) }, l)));
}

const copyButton = (command, label) => h("button.btn.sm.ghost.link", { "data-act": "cmdcopy", "data-i": command, title: `${command}, for a terminal` }, icon("copy"), label);

/** History stored at: the database path, and for the repo shown, Clear history, which asks first. */
function historyRow() {
  const path = data.health ? h("span.mono", null, data.health.database.path) : "…";
  const name = repoName() || ui.scope?.repository || "this repo";
  if (ui.scope === null) return row("History stored at", [path, " · stays on this machine"], null);
  if (!clearAsk) return row("History stored at", [path, " · stays on this machine"], h("button.btn.sm.danger", { "data-act": "clearask" }, "Clear history"));
  return row("History stored at",
    h("span.confirm", null, "Clear every recorded session and violation for ", h("span.mono", null, name),
      "? Rules and settings stay. This cannot be undone. In a terminal this is ", h("code", null, "rerail reset"), "."),
    h("span.btns", null, h("button.btn.sm", { "data-act": "clearcancel", disabled: clearing }, "Cancel"),
      h("button.btn.sm.danger.solid", { "data-act": "cleargo", disabled: clearing }, clearing ? "Clearing…" : "Clear history")));
}

function general() {
  const installed = installedLabel(data.health?.installedAt);
  return h("section", null, h("div.ghead", null, h("h2", null, "Settings")),
    h("div.list", null,
      row("Theme", null, seg("theme", [["", "System"], ["light", "Light"], ["dark", "Dark"]], ui.theme)),
      row("Status line", "Rerail’s line in Claude Code’s footer: how many rules are on and how often it steered. Other coding agents have no footer line.",
        data.presence === null ? h("span.val", null, "…") : seg("presence", [["compact", "Show"], ["silent", "Do not show"]], data.presence)),
      installed === null ? null : row("Installed", null, h("span.val", null, installed)),
      historyRow(),
      ui.scope === null ? null : row("Follow .gitignore",
        ["Files git ignores (", h("code", null, ".gitignore"), ", ", h("code", null, ".git/info/exclude"), ", your global excludes) are not checked in ",
          repoName() || "this repo", ". Applies to sessions started after the change."],
        data.followGitignore === null ? h("span.val", null, "…") : seg("gitignore", [["on", "On"], ["off", "Off"]], data.followGitignore ? "on" : "off"))));
}

/** One repo: its name (a link to it unless the page shows it), how many rules it has, and On / Off where it has any. */
function repoRow(repo) {
  const current = repo.repositoryKey === data.repository?.repositoryKey;
  const counts = repoCounts(repo);
  const tag = repoTag(counts);
  const name = current ? repo.name : h("button.btn.sm.ghost.link", { "data-act": "pickrepo", "data-i": repo.repositoryKey, title: "Show this repo’s rules" }, repo.name);
  const sub = [h("span.mono", null, repo.repositoryKey), ` · ${fmt(repo.checkoutCount)} ${plural(repo.checkoutCount, "checkout")}`];
  const right = counts === null || counts.total === 0 ? null : h("span.swl", null, h("span", null, repo.started ? "On" : "Off"),
    h("button.sw", { role: "switch", "aria-checked": String(repo.started), "aria-label": `Rerail in ${repo.name}`, "data-act": repo.started ? "repostop" : "repostart", "data-i": repo.repositoryKey }));
  return row([name, h("span.n", null, tag ?? "…")], sub, right, { "data-repo": repo.repositoryKey });
}

function repos() {
  return h("section", null,
    h("div.ghead", null, h("h2", null, "Repos"), h("p", null, "Rerail is installed once on this computer. Each repo has its own rules; switch a repo off to pause Rerail there. Its rules and history are kept.")),
    h("div.list", null, data.repositories.length ? data.repositories.map(repoRow) : h("div.empty", null, h("p", null, "Rerail has not seen a repo yet."))));
}

function agentRow(agent) {
  const status = agentStatus(agent);
  return h(`div.agent${status.tone === "off" ? ".off" : ""}`, { "data-agent": agent.id },
    h("div.rt2", null, h("span", null, agent.name), h(`span.st${status.tone && status.tone !== "off" ? `.${status.tone}` : ""}`, null, status.state),
      status.problem ? h("span.tip", null, status.problem) : null),
    status.command === null ? null : copyButton(status.command.text, status.command.label));
}

function agents() {
  const head = lead => h("div.ghead", null, h("h2", null, "Coding agents"), h("p", null, lead));
  if (data.agentsError !== null) return h("section", null, head(`Couldn’t check the coding agents: ${data.agentsError}`));
  if (data.agents === null) return h("section", null, head("Checking…"));
  const connected = data.agents.filter(a => a.connected && !a.problem).map(a => a.name);
  return h("section", null, head(connected.length ? `Connected to ${andList(connected)}.` : "No coding agent connected yet."),
    h("div.list.agents", null, data.agents.map(agentRow)));
}

function uninstall() {
  return h("section", null, h("div.ghead", null, h("h2", null, "Uninstall")),
    h("div.list", null, row("Remove Rerail from this computer",
      ["Run ", h("code", null, UNINSTALL_COMMAND), " in a terminal. Rules and history are kept in a backup."],
      copyButton(UNINSTALL_COMMAND, "Copy uninstall command"))));
}

/** After each poll, read every repo's rule count again (one 304 each when nothing changed). */
function refreshCounts() {
  if (data.updatedAt === 0 || countsRound === data.updatedAt) return;
  countsRound = data.updatedAt;
  loadRepoRuleCounts(data.repositories.map(r => r.repositoryKey)).then(changed => { if (changed) invalidate(); });
}

export function render(root) {
  refreshCounts();
  const counts = data.repositories.map(repoCounts);
  const sig = JSON.stringify([ui.theme, data.presence, ui.scope, data.followGitignore, data.health?.installedAt, data.health?.database.path,
    data.repositories, data.repository?.repositoryKey, data.repository?.name, counts, data.agents, data.agentsError, clearAsk, clearing]);
  setContent(root, sig, () => h("div.settings", null, general(), repos(), agents(), uninstall()));
}

/** The coding agents, the status line and Follow .gitignore are read on demand, each time Settings opens. */
export function enter() {
  clearAsk = false;
  countsRound = -1;
  data.agentsError = null;
  loadAgents().catch(error => { data.agentsError = error.message; }).then(invalidate);
  loadPresence().then(invalidate, error => toast(failed("Couldn’t load the status line setting", error)));
  if (ui.scope !== null) loadFollowGitignore().then(invalidate, error => toast(failed("Couldn’t load Follow .gitignore", error)));
}

async function savePresence(presence) {
  const was = data.presence;
  data.presence = presence; invalidate();
  try {
    await send("PUT", "/api/presentation-preferences", { repository: "", host: "", presence });
    toast(presence === "silent" ? "Status line hidden from the next turn." : "Status line shown from the next turn.");
  } catch (error) { data.presence = was; toast(failed("Couldn’t change the status line setting", error)); }
  invalidate();
}

/** "Cleared 12 sessions from storefront." plus the ones still running, which will keep recording. */
function clearedMessage({ sessions, openSessions }, name) {
  const running = openSessions === 0 ? "" : ` ${fmt(openSessions)} still running will keep recording.`;
  return `Cleared ${fmt(sessions)} ${plural(sessions, "session")} from ${name}.${running}`;
}

/** Clear the shown repo's history (`rerail reset`), then read everything of it again. */
async function clearHistory() {
  if (clearing) return;
  const scope = captureScope(), name = repoName() || "this repo";
  clearing = true; invalidate();
  try {
    const { body } = await send("POST", "/api/repository/reset", scope.target);
    toast(clearedMessage(body, name));
    if (inScope(scope)) {
      changeScope(ui.scope);
      loadFollowGitignore().then(invalidate, () => {});
    }
  } catch (error) {
    toast(failed(`Couldn’t clear history for ${name}`, error));
  }
  clearing = false; clearAsk = false;
  invalidate();
}

export const actions = {
  theme: v => applyTheme(v),
  presence: v => savePresence(v),
  gitignore: v => saveFollowGitignore(v === "on", { toast, invalidate }),
  clearask: () => { clearAsk = true; invalidate(); },
  clearcancel: () => { clearAsk = false; invalidate(); },
  cleargo: () => clearHistory(),
};
