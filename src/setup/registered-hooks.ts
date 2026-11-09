import { randomUUID } from "node:crypto";
import { once } from "node:events";
import { existsSync, readFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { isAbsolute, join } from "node:path";
import { spawn } from "../engine/child-process.js";
import { removeScratch } from "../engine/file-replace.js";
import { resolveExecutable } from "../runtime/npm.js";

/** Rerail's hook scripts, as a registration names them in a hook's command or arguments. */
const RERAIL_HOOK_SCRIPT = /[\\/]dist[\\/](?:harness[\\/]hook-launcher|hosts[\\/]claude[\\/]claude-(?:hook|status))\.js$/u;

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };

/** Every string inside `value`, depth first. */
function strings(value: Json | undefined, found: string[] = []): string[] {
  if (typeof value === "string") found.push(value);
  else if (Array.isArray(value)) for (const item of value) strings(item, found);
  else if (value !== null && typeof value === "object") for (const item of Object.values(value)) strings(item, found);
  return found;
}

/** The absolute paths a command line names, quoted or not. */
function pathsIn(text: string): string[] {
  const words = text.match(/"[^"]+"|'[^']+'|\S+/gu) ?? [];
  return words.map(word => word.replace(/^["']|["']$/gu, "")).filter(word => isAbsolute(word));
}

/**
 * The Rerail hook and status-line scripts Claude Code's user settings run that are missing on disk: a registration
 * that points at a removed install or folder makes every hook fail before any Rerail code runs, so nothing records and
 * nothing Rerail runs can say so. Empty when the settings name none, or cannot be read.
 */
export function missingClaudeHookScripts(home: string): string[] {
  let settings: Json;
  try {
    settings = JSON.parse(readFileSync(join(home, ".claude", "settings.json"), "utf8")) as Json;
  } catch {
    return [];
  }
  const record = settings !== null && typeof settings === "object" && !Array.isArray(settings) ? settings : {};
  const named = strings([record.hooks ?? null, record.statusLine ?? null]).flatMap(pathsIn);
  return [...new Set(named.filter(path => RERAIL_HOOK_SCRIPT.test(path) && !existsSync(path)))];
}

/** A Claude Code hook entry as settings.json holds it: a command line, or a command with its own arguments. */
interface HookEntry {
  command: string;
  args?: readonly string[];
}

/** The first SessionStart hook in Claude Code's user settings that runs Rerail's Claude hook. */
function rerailSessionStartHook(home: string): HookEntry | undefined {
  let settings: Json;
  try {
    settings = JSON.parse(readFileSync(join(home, ".claude", "settings.json"), "utf8")) as Json;
  } catch {
    return undefined;
  }
  const hooks = settings !== null && typeof settings === "object" && !Array.isArray(settings) ? settings.hooks : undefined;
  const groups = hooks !== null && typeof hooks === "object" && !Array.isArray(hooks) ? hooks.SessionStart : undefined;
  for (const group of Array.isArray(groups) ? groups : []) {
    const entries = group !== null && typeof group === "object" && !Array.isArray(group) ? group.hooks : undefined;
    for (const entry of Array.isArray(entries) ? entries : []) {
      if (entry === null || typeof entry !== "object" || Array.isArray(entry) || typeof entry.command !== "string") continue;
      const args = Array.isArray(entry.args) ? entry.args.filter((arg): arg is string => typeof arg === "string") : undefined;
      if (strings([entry.command, ...(args ?? [])]).some(text => /claude-hook\.js/u.test(text))) {
        return args === undefined ? { command: entry.command } : { command: entry.command, args };
      }
    }
  }
  return undefined;
}

const HOOK_CHECK_TIMEOUT_MS = 30_000;

/**
 * Runs Rerail's SessionStart hook exactly as Claude Code's user settings register it, with a session start in an empty
 * folder where Rerail is not started, and says what is wrong when it does not answer as a working hook does: exit 0, a
 * JSON object on stdout and no `rerail hook:` failure on stderr. Undefined when it works, or when no Rerail hook is
 * registered. Set-up runs it after connecting Claude Code, so a registration that cannot run fails set-up instead of
 * leaving every session unrecorded.
 */
export async function runRegisteredClaudeHook(home: string, env: NodeJS.ProcessEnv = process.env): Promise<string | undefined> {
  const hook = rerailSessionStartHook(home);
  if (hook === undefined) return undefined;
  const folder = await mkdtemp(join(tmpdir(), "rerail-hook-check-"));
  try {
    const payload = { session_id: `rerail-setup-check-${randomUUID()}`, cwd: folder, hook_event_name: "SessionStart", source: "startup" };
    const child = hook.args === undefined
      ? spawn(hook.command, { shell: true, cwd: folder, env: { ...env, CLAUDE_PROJECT_DIR: folder } })
      : spawn(registeredProgram(hook.command, env), [...hook.args], { cwd: folder, env: { ...env, CLAUDE_PROJECT_DIR: folder } });
    let stdout = "";
    let stderr = "";
    child.stdout.setEncoding("utf8").on("data", (chunk: string) => { stdout += chunk; });
    child.stderr.setEncoding("utf8").on("data", (chunk: string) => { stderr += chunk; });
    child.stdin.end(JSON.stringify(payload));
    const timer = setTimeout(() => child.kill(), HOOK_CHECK_TIMEOUT_MS);
    const [code] = await Promise.race([once(child, "exit"), once(child, "error").then(([error]) => { throw error; })]) as [number | null];
    clearTimeout(timer);
    const failure = stderr.split(/\r?\n/u).find(line => line.startsWith("rerail hook:"));
    if (code !== 0) return `Claude Code's Rerail hook exited ${String(code)}: ${(failure ?? stderr.trim()) || "no output"}`;
    if (failure !== undefined) return `Claude Code's Rerail hook failed: ${failure}`;
    try {
      const answer = JSON.parse(stdout) as unknown;
      if (answer === null || typeof answer !== "object" || Array.isArray(answer)) return "Claude Code's Rerail hook answered with no JSON object";
    } catch {
      return "Claude Code's Rerail hook answered with no JSON object";
    }
    return undefined;
  } catch (error) {
    return `Claude Code's Rerail hook could not run: ${error instanceof Error ? error.message : String(error)}`;
  } finally {
    await removeScratch(folder);
  }
}

/**
 * The program an exec-form entry names, found as a spawn without a shell finds it: the bare `node` Rerail registers on
 * Windows, resolved through PATH to its absolute path, since the shared child-process helper starts no program there
 * from the current folder. A name PATH cannot resolve stays as written, and the spawn's refusal is the failure reported.
 */
function registeredProgram(command: string, env: NodeJS.ProcessEnv): string {
  return resolveExecutable(command, env, process.platform, { binaryOnly: true }) ?? command;
}
