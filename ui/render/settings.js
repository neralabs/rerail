/**
 * Settings (W6). General: theme, rule panel width, the status line, history and the daemon. Repos:
 * each Started or Stopped, with a switch. Coding agents: which are connected. Uninstall: the command
 * and where the backup goes. Rerail starts per repo; there is no every-repo setting.
 */
import { h, setContent, icon } from "../dom.js";
import { andList, fmt, plural } from "../format.js";
import { data, ui } from "../state.js";

const UNINSTALL_COMMAND = "rerail install --remove";

function row(label, sub, right) {
  return h("div.srow2", null, h("div.rt", null, label, sub ? h("span.sub", null, sub) : null), right);
}

function seg(act, options, current) {
  return h("div.rangectl", { role: "group" }, options.map(([v, l]) => h("button", { "data-act": act, "data-i": v, "aria-pressed": String(current === v) }, l)));
}

const copyButton = (command, label) => h("button.btn.sm.ghost.link", { "data-act": "cmdcopy", "data-i": command, title: command }, icon("copy"), label);

function general() {
  const health = data.health, live = data.loaded && !data.disconnected;
  return h("section", null, h("div.ghead", null, h("h2", null, "General")),
    h("div.list", null,
      row("Theme", "System follows your OS or editor.", seg("theme", [["", "System"], ["light", "Light"], ["dark", "Dark"]], ui.theme)),
      row("Rule panel", "Wide gives long titles and rule pack lists more room.", seg("pwidth", [["0", "Default"], ["1", "Wide"]], ui.wide ? "1" : "0")),
      row("Status line", "Rerail’s line in your coding agent’s footer: how many rules are on, and how often it steered.",
        data.presence === null ? h("span.val", null, "…") : seg("presence", [["compact", "Show"], ["silent", "Do not show"]], data.presence)),
      row("History", health ? ["Kept in ", h("span.mono", null, health.database.path), " · nothing leaves your computer"] : "Nothing leaves your computer", null),
      row("Daemon", health ? [h("span.mono", null, location.origin), ` · v${health.version}`] : h("span.mono", null, location.origin),
        h("span.val", null, live ? "Running" : data.loaded ? "Not answering" : "Connecting…"))));
}

/** One repository (`GET /api/repositories`): set up or not, and a switch that starts or stops every checkout of it. */
function repoRow(repo) {
  const current = repo.repositoryKey === data.repository?.repositoryKey;
  const on = current ? [...data.rules.values()].filter(r => r.enabled).length : null;
  const checkouts = `${fmt(repo.checkoutCount)} ${plural(repo.checkoutCount, "checkout")}`;
  const detail = !repo.setUp ? "not set up yet" : current ? `${fmt(on)} ${plural(on, "rule")} on` : "set up";
  const sub = [h("span.mono", null, repo.repositoryKey), ` · ${checkouts} · ${detail}`,
    current ? h("span.tip", null, h("b", null, "Tip"), "Stopped means nothing is audited or logged here; rules and history are kept. The switch acts on every checkout of the repo; from a session, ",
      h("code", null, "/rerail-stop"), " and ", h("code", null, "/rerail-start"), " act on that worktree.") : null,
    !repo.setUp ? h("span.tip", null, h("b", null, "Tip"), "Type ", h("code", null, "/rerail-start"), " in a coding agent session in this repo to set it up.") : null];
  const right = h("span.swl", null, h("span", null, repo.started ? "Started" : "Stopped"),
    h("button.sw", { role: "switch", "aria-checked": String(repo.started), "aria-label": `Rerail in ${repo.name}`, "data-act": repo.started ? "repostop" : "repostart", "data-i": repo.repositoryKey }));
  const name = current ? repo.name : h("button.btn.sm.ghost.link", { "data-act": "pickrepo", "data-i": repo.repositoryKey, title: "Show this repo’s rules" }, repo.name);
  return row([name, current ? h("span.n", null, "this dashboard") : null], sub, right);
}

function repos() {
  return h("section", null,
    h("div.ghead", null, h("h2", null, "Repos"), h("p", null, "Rerail is installed once and started per repo. Each repo has its own rule packs; rules you create are kept once on this computer and added to the repos that use them.")),
    h("div.list", null, data.repositories.length ? data.repositories.map(repoRow) : h("div.empty", null, h("p", null, "No repo has been set up yet."))));
}

function agentRow(agent) {
  if (agent.connected) return h("div.agent", null, h("div.rt2", null, h("span", null, agent.name)), h("span.st.on", null, "Connected"));
  if (!agent.found) return h("div.agent", null, h("div.rt2", null, h("span", null, agent.name)), h("span.st", null, "Not found on this computer"));
  if (agent.problem) {
    return h("div.agent", null,
      h("div.rt2", null, h("span", null, agent.name), h("span.tip", null, h("b", null, "Problem"), agent.problem)),
      h("span.st", null, "Couldn’t check"));
  }
  const command = `rerail install --agents ${agent.id}`;
  return h("div.agent", null,
    h("div.rt2", null, h("span", null, agent.name), h("span.tip", null, h("b", null, "Tip"), "In a terminal, ", h("code", null, command), ` connects Rerail to ${agent.name}.`)),
    h("span.stack", null, h("span.st", null, "On this computer, not connected"), copyButton(command, "Copy command")));
}

function agents() {
  const list = data.agents;
  if (data.agentsError !== null) return h("section", null, h("div.ghead", null, h("h2", null, "Coding agents"), h("p", null, `Couldn’t check the coding agents: ${data.agentsError}`)));
  if (list === null) return h("section", null, h("div.ghead", null, h("h2", null, "Coding agents"), h("p", null, "Checking…")));
  const connected = list.filter(a => a.connected).map(a => a.name);
  const found = list.filter(a => a.found && !a.connected).map(a => a.name);
  const lead = [connected.length ? `Connected to ${andList(connected)}.` : "No coding agent connected yet.",
    found.length ? ` ${andList(found)} ${found.length === 1 ? "is" : "are"} on this computer but not connected.` : ""].join("");
  return h("section", null, h("div.ghead", null, h("h2", null, "Coding agents"), h("p", null, lead)), h("div.list.agents", null, list.map(agentRow)));
}

function uninstall() {
  return h("section", null, h("div.ghead", null, h("h2", null, "Uninstall")),
    h("div.list", null, row("Remove Rerail from this computer",
      ["Run ", h("code", null, UNINSTALL_COMMAND), " in a terminal. It stops the daemon and removes Rerail from every coding agent. Your rules and history move to a backup folder in your home folder, ",
        h("code", null, "~/.rerail-clean-<date>"), ", so installing again can restore them."],
      copyButton(UNINSTALL_COMMAND, "Copy uninstall command"))));
}

export function renderSettings(root) {
  const sig = JSON.stringify([ui.theme, ui.wide, data.presence, data.health?.version, data.health?.database.path, data.loaded, data.disconnected,
    data.repositories, data.repository?.repositoryKey, [...data.rules.values()].filter(r => r.enabled).length, data.agents, data.agentsError]);
  setContent(root, sig, () => h("div.settings", null, general(), repos(), agents(), uninstall()));
}
