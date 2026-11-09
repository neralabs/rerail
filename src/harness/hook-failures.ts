import { appendFileSync, readFileSync, renameSync, statSync } from "node:fs";
import { join } from "node:path";
import { PACKAGE_ROOT } from "../engine/package-root.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { makeRerailHome } from "../engine/storage/private-files.js";

/**
 * In the Rerail home: one JSON line per hook event Rerail could not record, so `rerail status` can say that hooks run
 * but nothing is recorded, and why. A hook's own stderr reaches no one: coding agents keep it out of the session.
 */
export const HOOK_FAILURES_FILE = "hook-failures.jsonl";

/** Past this size the file is moved to `<name>.1`, replacing the one before; a failing hook writes on every event. */
const MAX_FAILURE_BYTES = 256 * 1024;

export interface HookFailure {
  /** Epoch ms. */
  at: number;
  /** The coding agent, as Rerail names its hosts (`claude-code`, `codex`, …). */
  host: string;
  /** The agent's own name for the event, such as `MessageDisplay`. */
  event: string;
  /** The agent and its version as it tells its hooks (Claude Code's `AI_AGENT`), when it does. */
  agent: string | null;
  /** The Rerail version that ran the hook. */
  rerail: string;
  reason: string;
}

function rerailVersion(): string {
  try {
    const manifest = JSON.parse(readFileSync(join(PACKAGE_ROOT, "package.json"), "utf8")) as { version?: unknown };
    return typeof manifest.version === "string" ? manifest.version : "unknown";
  } catch {
    return "unknown";
  }
}

export interface HookFailureContext {
  home?: string;
  now?: number;
  env?: NodeJS.ProcessEnv;
}

/** Appends one failure. Never throws: a hook must not break the session over its own bookkeeping. */
export function recordHookFailure(
  failure: Pick<HookFailure, "host" | "event" | "reason">,
  { home = defaultRerailHome(), now = Date.now(), env = process.env }: HookFailureContext = {},
): void {
  try {
    makeRerailHome(home);
    const path = join(home, HOOK_FAILURES_FILE);
    const size = (() => {
      try {
        return statSync(path).size;
      } catch {
        return 0;
      }
    })();
    if (size > MAX_FAILURE_BYTES) renameSync(path, `${path}.1`);
    const line: HookFailure = { at: now, ...failure, agent: env.AI_AGENT ?? null, rerail: rerailVersion() };
    appendFileSync(path, `${JSON.stringify(line)}\n`, { mode: 0o600 });
  } catch {
    // Nowhere left to say it.
  }
}

/** How far back `rerail status` looks for hook failures. */
export const HOOK_FAILURE_WINDOW_MS = 24 * 60 * 60 * 1000;

const plural = (count: number, one: string, many: string): string => `${count} ${count === 1 ? one : many}`;

/**
 * What `rerail status` says of `failures` (oldest first, from the last `HOOK_FAILURE_WINDOW_MS`), given when the newest
 * recorded event was observed (`lastRecordedAt`, epoch ms; null when nothing was ever recorded). When every failure is
 * newer than the last recorded event, hooks run but nothing is recorded, and the line leads with that.
 */
export function hookFailureLines(failures: readonly HookFailure[], lastRecordedAt: number | null): string[] {
  const last = failures.at(-1);
  if (last === undefined) return [];
  const since = lastRecordedAt === null ? failures : failures.filter(failure => failure.at > lastRecordedAt);
  const from = last.agent ?? last.host;
  const detail = `the last, ${last.event} from ${from} with Rerail ${last.rerail} at ${new Date(last.at).toISOString()}: ${last.reason}`;
  if (since.length === failures.length) {
    return [`Hooks run but nothing was recorded since ${new Date(failures[0]!.at).toISOString()}: ${plural(failures.length, "hook event", "hook events")} failed; ${detail}.`];
  }
  return [`Hooks: ${plural(failures.length, "hook event", "hook events")} could not be recorded in the last 24 hours; ${detail}.`];
}

/** The failures recorded since `since` (epoch ms), oldest first; none when the file is missing or unreadable. */
export function readHookFailures(home: string, since: number): HookFailure[] {
  let text: string;
  try {
    text = readFileSync(join(home, HOOK_FAILURES_FILE), "utf8");
  } catch {
    return [];
  }
  const failures: HookFailure[] = [];
  for (const line of text.split("\n")) {
    if (line.trim() === "") continue;
    try {
      const value = JSON.parse(line) as Partial<HookFailure>;
      if (typeof value.at === "number" && value.at >= since && typeof value.event === "string" && typeof value.reason === "string") {
        failures.push({ at: value.at, host: value.host ?? "unknown", event: value.event, agent: value.agent ?? null, rerail: value.rerail ?? "unknown", reason: value.reason });
      }
    } catch {
      // A line cut short by a concurrent write.
    }
  }
  return failures;
}
