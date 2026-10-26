import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { join } from "node:path";
import type { CodingAgentState } from "../engine/adapter-contract.js";
import {
  detectHarnesses,
  HARNESSES,
  registerHarnesses,
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
