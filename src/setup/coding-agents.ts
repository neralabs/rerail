import { existsSync, readFileSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import type { CodingAgentState } from "../engine/adapter-contract.js";
import {
  CODEX_TRUST_NOTE,
  detectHarnesses,
  HARNESSES,
  loadsOmpPluginCopy,
  OMP_PLUGIN_NOTE,
  piHoldsRerailNpmSource,
  registerHarnesses,
  UnreadableConfigError,
  unregisterHarnesses,
  type Harness,
  type RegistrationChange,
} from "./harness-registration.js";

/** The names people see for each coding agent `--agents` takes. */
export const CODING_AGENT_NAMES: Readonly<Record<Harness, string>> = {
  omp: "OMP",
  pi: "Pi",
  "claude-code": "Claude Code",
  codex: "Codex",
  cursor: "Cursor",
};

export type { CodingAgentState };

/**
 * Coding agent config lives under the user's home (`~/.cursor`, `~/.claude`, …), not under the Rerail home: the home
 * the coding agents read, which is `HOME` on macOS and Linux and the profile folder (`USERPROFILE`) on Windows, where
 * Git Bash's own `HOME` may name another folder.
 */
export function codingAgentHome(): string {
  return homedir();
}

/** Registrations point at `<rerail home>/current` once set-up has run; before that, at the running package. */
export function registrationRoot(rerailHome: string, packageRoot: string): string {
  const current = join(rerailHome, "current");
  return existsSync(join(current, "package.json")) ? current : packageRoot;
}

/**
 * Every coding agent Rerail works with: whether it is on this computer, and whether Rerail is connected
 * to it (a dry run of registering it against `root` would write nothing). One dry run per coding agent,
 * so a config that cannot be read marks only that one, with the reason. Changes nothing.
 */
export async function codingAgentStates(root: string, home: string): Promise<CodingAgentState[]> {
  const found = new Set(detectHarnesses(home));
  const states: CodingAgentState[] = [];
  for (const id of HARNESSES) {
    const base = { id, name: CODING_AGENT_NAMES[id], found: found.has(id) };
    states.push(base.found ? { ...base, ...await connection(root, home, id) } : { ...base, connected: false, problem: null });
  }
  return states;
}

async function connection(root: string, home: string, id: Harness): Promise<Pick<CodingAgentState, "connected" | "problem">> {
  try {
    const plan = await registerHarnesses({ root, home, harnesses: [id], dryRun: true });
    return { connected: plan.changes.every(change => change.harness !== id || change.action === "unchanged"), problem: null };
  } catch (error) {
    return { connected: false, problem: error instanceof Error ? error.message : String(error) };
  }
}

/**
 * Whether coding agent `id` holds any registration of `root`: a dry run of removing Rerail from it would change
 * something. An agent missing part of what this version registers (a command added since) counts, and so does Pi with
 * only Rerail's npm source from Pi's gallery when `root` is the managed install, which that source stands in for. One
 * the person never connected, or connected to another root, does not. Changes nothing; a configuration that cannot be
 * read throws.
 */
async function isRegistered(root: string, home: string, id: Harness): Promise<boolean> {
  const plan = await unregisterHarnesses({ root, home, harnesses: [id], dryRun: true });
  return plan.changes.some(change => change.harness === id && change.action !== "unchanged" && change.action !== "skipped");
}

export interface RefreshProblem {
  agent: Harness;
  message: string;
}

export interface RefreshResult {
  /** The coding agents re-registered: those that held a registration of the root. */
  agents: Harness[];
  changes: RegistrationChange[];
  notes: string[];
  /** Connected coding agents whose registration could not be written, with the reason; the others are refreshed. */
  failures: RefreshProblem[];
  /**
   * Coding agents left alone because their configuration could not be read to tell whether Rerail is connected:
   * one never connected must not fail an update, so these are reported, not failures.
   */
  unchecked: RefreshProblem[];
}

/**
 * Registers `root` again with every coding agent already registered with it, so a new version's commands, skills and
 * hooks arrive and the ones it retired go. One coding agent at a time, so a configuration that cannot be read stops
 * only that one. Agents with no registration of `root` are left alone.
 */
export async function refreshCodingAgents(root: string, home: string): Promise<RefreshResult> {
  const result: RefreshResult = { agents: [], changes: [], notes: [], failures: [], unchecked: [] };
  const message = (error: unknown): string => error instanceof Error ? error.message : String(error);
  for (const id of detectHarnesses(home)) {
    try {
      if (!(await isRegistered(root, home, id))) continue;
    } catch (error) {
      result.unchecked.push({ agent: id, message: message(error) });
      continue;
    }
    try {
      const report = await registerHarnesses({ root, home, harnesses: [id] });
      result.agents.push(id);
      result.changes.push(...report.changes.filter(change => change.harness === id));
      for (const note of report.notes) if (!result.notes.includes(note)) result.notes.push(note);
    } catch (error) {
      result.failures.push({ agent: id, message: message(error) });
    }
  }
  return result;
}

export interface ConnectProblem extends RefreshProblem {
  /** The agent's config file when it is not plain JSON; `message` is then the parser's reason. */
  file?: string;
}

export interface ConnectResult {
  /** The coding agents named whose config this call changed: not connected, or not wholly, before. */
  changed: Harness[];
  /**
   * Every coding agent on this computer that holds a registration of the root now, these included: one whose
   * registration is out of date (its hooks name a Node that has moved since, say) is still connected, its hooks
   * still run, and `rerail update` brings it up to date.
   */
  all: Harness[];
  /**
   * The coding agents named, and Pi when its npm source was to be swapped, that could not be registered, with the
   * reason; their config is left as it was.
   */
  failures: ConnectProblem[];
  /** Codex's hooks were written: Codex runs them only once the person trusts them with `/hooks`. */
  codexTrust: boolean;
  /** OMP also loads its own plugin install of Rerail (`OMP_PLUGIN_NOTE`): the person removes it with `omp plugin uninstall`. */
  ompPlugin: boolean;
}

/**
 * Set-up's connection of `agents` to `root`, one coding agent at a time, as `refreshCodingAgents` does: a config that
 * cannot be read (a comment or trailing comma in a JSON file) stops only its own agent, untouched, and the others are
 * connected. `workspaceDir`: the repository the person said yes to, for Claude Code's project status line.
 *
 * A connected agent not named is otherwise left as it is, out-of-date registration and all, for `rerail update`, but for
 * two copies of Rerail loading in it: Pi is registered again while its settings also hold Rerail's npm source (a
 * `pi install npm:@neralabs/rerail` since), which swaps that source for `root`, and OMP's plugin install is looked for
 * whenever OMP is connected.
 */
export async function connectCodingAgents(root: string, home: string, agents: readonly Harness[], workspaceDir?: string): Promise<ConnectResult> {
  const result: ConnectResult = { changed: [], all: [], failures: [], codexTrust: false, ompPlugin: false };
  for (const id of [...agents, ...await piToSwap(root, home, agents)]) {
    try {
      const report = await registerHarnesses({ root, home, harnesses: [id], ...(workspaceDir === undefined ? {} : { workspaceDir }) });
      if (agents.includes(id) && report.changes.some(change => change.harness === id && (change.action === "created" || change.action === "updated"))) result.changed.push(id);
      if (report.notes.includes(CODEX_TRUST_NOTE)) result.codexTrust = true;
      if (report.notes.includes(OMP_PLUGIN_NOTE)) result.ompPlugin = true;
    } catch (error) {
      result.failures.push(error instanceof UnreadableConfigError
        ? { agent: id, message: error.reason, file: error.path }
        : { agent: id, message: error instanceof Error ? error.message : String(error) });
    }
  }
  for (const id of detectHarnesses(home)) {
    try {
      if (await isRegistered(root, home, id)) result.all.push(id);
    } catch {
      // A config that cannot be read connects nothing; set-up names it when it was asked to connect it.
    }
  }
  if (!result.ompPlugin && !agents.includes("omp") && result.all.includes("omp")) result.ompPlugin = await loadsOmpPluginCopy(root, home);
  return result;
}

/** `["pi"]` when Pi, not among `agents`, is connected to `root` and its settings also hold Rerail's npm source. */
async function piToSwap(root: string, home: string, agents: readonly Harness[]): Promise<Harness[]> {
  if (agents.includes("pi") || !detectHarnesses(home).includes("pi")) return [];
  try {
    return await piHoldsRerailNpmSource(home) && await isRegistered(root, home, "pi") ? ["pi"] : [];
  } catch {
    // A config that cannot be read is left as it is, as for any agent not named.
    return [];
  }
}

/**
 * In the Rerail home: `{ "seen": [<agent id>, …] }`, every coding agent set-up or `rerail install` found on this
 * computer or was named, but one it could not connect. Set-up run again in a repository that is set up connects only
 * the agents found since, so it never undoes `rerail install --remove --agents <id>` or an earlier `--agents` that
 * left one out, and does retry an agent whose config it could not read.
 */
const SEEN_AGENTS_FILE = "coding-agents.json";

/**
 * The coding agents recorded as seen, or undefined when there is no record to go by: none written yet (an install from
 * before the record existed, whose removals and `--agents` choices it never saw) or one that cannot be read. Set-up run
 * again then connects no agent rather than every one found, and the record starts from what is found now.
 */
export function seenCodingAgents(rerailHome: string): Harness[] | undefined {
  try {
    const { seen } = JSON.parse(readFileSync(join(rerailHome, SEEN_AGENTS_FILE), "utf8")) as { seen?: unknown };
    return Array.isArray(seen) ? HARNESSES.filter(id => seen.includes(id)) : undefined;
  } catch {
    return undefined;
  }
}

async function writeSeenCodingAgents(rerailHome: string, seen: ReadonlySet<Harness>): Promise<void> {
  await mkdir(rerailHome, { recursive: true, mode: 0o700 });
  await writeFile(join(rerailHome, SEEN_AGENTS_FILE), `${JSON.stringify({ seen: HARNESSES.filter(id => seen.has(id)) }, null, 2)}\n`, { mode: 0o600 });
}

/** Adds `agents` to the coding agents recorded as seen, starting the record if there is none; writes nothing when each is recorded already. */
export async function recordSeenCodingAgents(rerailHome: string, agents: readonly Harness[]): Promise<void> {
  const before = seenCodingAgents(rerailHome);
  const seen = new Set([...before ?? [], ...agents]);
  if (before !== undefined && seen.size === before.length) return;
  await writeSeenCodingAgents(rerailHome, seen);
}

/**
 * Takes `agents` out of the record: set-up could not connect them, so set-up run again tries them once more, even one
 * `rerail install` recorded earlier.
 */
export async function forgetSeenCodingAgents(rerailHome: string, agents: readonly Harness[]): Promise<void> {
  const before = seenCodingAgents(rerailHome);
  if (before === undefined || !agents.some(id => before.includes(id))) return;
  await writeSeenCodingAgents(rerailHome, new Set(before.filter(id => !agents.includes(id))));
}
