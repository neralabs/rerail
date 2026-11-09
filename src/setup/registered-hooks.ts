import { existsSync, readFileSync } from "node:fs";
import { isAbsolute, join } from "node:path";

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
