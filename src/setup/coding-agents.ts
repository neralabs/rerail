import { existsSync, readFileSync } from "node:fs";
import { mkdir, writeFile } from "node:fs/promises";
import { homedir } from "node:os";
import { join } from "node:path";
import type { CodingAgentState } from "../engine/adapter-contract.js";
import {
  CODEX_TRUST_NOTE,
  detectHarnesses,
  HARNESSES,
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

/** Coding agent config lives under the user's home (`~/.cursor`, `~/.claude`, …), not under the Rerail home. */
export function codingAgentHome(): string {
  return process.env.HOME ?? homedir();
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
 * something. An agent missing part of what this version registers (a command added
 * since) counts, and one the person never connected, or connected to another root, does not. Changes nothing; a
 * configuration that cannot be read throws.
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
  /** Every coding agent on this computer that Rerail is connected to now, these included. */
  all: Harness[];
  /** The coding agents named that could not be connected, with the reason; their config is left as it was. */
  failures: ConnectProblem[];
  /** Codex's hooks were written: Codex runs them only once the person trusts them with `/hooks`. */
  codexTrust: boolean;
}

/**
 * Set-up's connection of `agents` to `root`, one coding agent at a time, as `refreshCodingAgents` does: a config that
 * cannot be read (a comment or trailing comma in a JSON file) stops only its own agent, untouched, and the others are
 * connected. `workspaceDir`: the repository the person said yes to, for Claude Code's project status line.
 */
export async function connectCodingAgents(root: string, home: string, agents: readonly Harness[], workspaceDir?: string): Promise<ConnectResult> {
  const result: ConnectResult = { changed: [], all: [], failures: [], codexTrust: false };
  for (const id of agents) {
    try {
      const report = await registerHarnesses({ root, home, harnesses: [id], ...(workspaceDir === undefined ? {} : { workspaceDir }) });
      if (report.changes.some(change => change.harness === id && (change.action === "created" || change.action === "updated"))) result.changed.push(id);
      if (report.notes.includes(CODEX_TRUST_NOTE)) result.codexTrust = true;
    } catch (error) {
      result.failures.push(error instanceof UnreadableConfigError
        ? { agent: id, message: error.reason, file: error.path }
        : { agent: id, message: error instanceof Error ? error.message : String(error) });
    }
  }
  for (const id of detectHarnesses(home)) {
    if ((await connection(root, home, id)).connected) result.all.push(id);
  }
  return result;
}

/**
 * In the Rerail home: `{ "seen": [<agent id>, …] }`, every coding agent set-up or `rerail install` found on this
 * computer or was named. Set-up run again in a repository that is set up connects only the agents found since, so
 * it never undoes `rerail install --remove --agents <id>` or an earlier `--agents` that left one out.
 */
const SEEN_AGENTS_FILE = "coding-agents.json";

/** The coding agents recorded as seen; none when nothing is recorded yet (or the record cannot be read). */
export function seenCodingAgents(rerailHome: string): Harness[] {
  try {
    const { seen } = JSON.parse(readFileSync(join(rerailHome, SEEN_AGENTS_FILE), "utf8")) as { seen?: unknown };
    return Array.isArray(seen) ? HARNESSES.filter(id => seen.includes(id)) : [];
  } catch {
    return [];
  }
}

/** Adds `agents` to the coding agents recorded as seen; writes nothing when each is recorded already. */
export async function recordSeenCodingAgents(rerailHome: string, agents: readonly Harness[]): Promise<void> {
  const before = seenCodingAgents(rerailHome);
  const seen = new Set([...before, ...agents]);
  if (seen.size === before.length) return;
  await mkdir(rerailHome, { recursive: true, mode: 0o700 });
  await writeFile(join(rerailHome, SEEN_AGENTS_FILE), `${JSON.stringify({ seen: HARNESSES.filter(id => seen.has(id)) }, null, 2)}\n`, { mode: 0o600 });
}
