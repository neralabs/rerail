import { existsSync } from "node:fs";
import { homedir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { detectHarnesses, HARNESSES, registerHarnesses, type Harness } from "./harness-registration.js";

/** The names people see for each coding agent `--agents` takes. */
export const CODING_AGENT_NAMES: Readonly<Record<Harness, string>> = {
  omp: "OMP",
  pi: "Pi",
  "claude-code": "Claude Code",
  codex: "Codex",
  cursor: "Cursor",
};

export interface CodingAgentState {
  id: Harness;
  name: string;
  /** Its config folder exists under the user's home. */
  found: boolean;
  /** Registering it again would write nothing: Rerail's hooks and commands are in place. */
  connected: boolean;
  /** Why its connection could not be checked, such as a config file that is not valid JSON; null when it could. */
  problem: string | null;
}

/** Coding agent config lives under the user's home (`~/.cursor`, `~/.claude`, …), not under the Rerail home. */
export function codingAgentHome(): string {
  return process.env.HOME ?? homedir();
}

/** Registrations point at `<rerail home>/current` once set-up has run; before that, at the running package. */
export function registrationRoot(rerailHome: string, packageRoot: string): string {
  const current = join(rerailHome, "current");
  return existsSync(join(current, "package.json")) ? current : packageRoot;
}

/** The package this module runs from: `dist/` and `src/` both sit one level below it. */
export const RUNNING_PACKAGE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

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

/** The coding agents Rerail is connected to now, as `codingAgentStates` checks them. */
export async function connectedCodingAgents(root: string, home: string): Promise<Harness[]> {
  return (await codingAgentStates(root, home)).filter(state => state.connected).map(state => state.id);
}
