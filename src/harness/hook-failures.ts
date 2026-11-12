import { appendFileSync, readFileSync, statSync } from "node:fs";
import { join } from "node:path";
import { renameWithRetrySync } from "../engine/file-replace.js";
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

/** The log's files, the part moved aside first: oldest first. */
const LOG_FILES = [`${HOOK_FAILURES_FILE}.1`, HOOK_FAILURES_FILE] as const;

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
    if (size > MAX_FAILURE_BYTES) renameWithRetrySync(path, `${path}.1`);
    const line: HookFailure = { at: now, ...failure, agent: env.AI_AGENT ?? null, rerail: rerailVersion() };
    appendFileSync(path, `${JSON.stringify(line)}\n`, { mode: 0o600 });
  } catch {
    // Nowhere left to say it.
  }
}

/** How far back `rerail status` looks for hook failures. */
export const HOOK_FAILURE_WINDOW_MS = 24 * 60 * 60 * 1000;

const plural = (count: number, one: string, many: string): string => `${count} ${count === 1 ? one : many}`;

/** What the log holds of the failures since a given time. */
export interface HookFailureLog {
  /** Those failures, oldest first. */
  failures: HookFailure[];
  /**
   * When the log was moved aside and keeps nothing as old as the given time, failures before its oldest may be gone:
   * that oldest failure's time, epoch ms, as far back as the log can count. Null when the log reaches back to the given
   * time.
   */
  keptSince: number | null;
}

/**
 * What `rerail status` says of the log's failures from the last `HOOK_FAILURE_WINDOW_MS`, given when the newest
 * recorded event was observed (`lastRecordedAt`, epoch ms; null when nothing was ever recorded). When every failure is
 * newer than the last recorded event, hooks run but nothing is recorded, and the line leads with that.
 */
export function hookFailureLines({ failures, keptSince }: HookFailureLog, lastRecordedAt: number | null): string[] {
  const last = failures.at(-1);
  if (last === undefined) return [];
  const since = lastRecordedAt === null ? failures : failures.filter(failure => failure.at > lastRecordedAt);
  const from = last.agent ?? last.host;
  const detail = `the last, ${last.event} from ${from} with Rerail ${last.rerail} at ${new Date(last.at).toISOString()}: ${last.reason}`;
  if (since.length === failures.length) {
    return [`Hooks run but nothing was recorded since ${new Date(failures[0]!.at).toISOString()}: ${plural(failures.length, "hook event", "hook events")} failed; ${detail}.`];
  }
  const period = keptSince === null ? "in the last 24 hours" : `since ${new Date(keptSince).toISOString()}, as far back as the log goes`;
  return [`Hooks: ${plural(failures.length, "hook event", "hook events")} could not be recorded ${period}; ${detail}.`];
}

/**
 * The failures recorded since `since` (epoch ms), in the part of the log moved aside and the part after it. A file
 * missing or unreadable holds none.
 */
export function readHookFailures(home: string, since: number): HookFailureLog {
  const kept: HookFailure[] = [];
  let movedAside = false;
  for (const name of LOG_FILES) {
    let text: string;
    try {
      text = readFileSync(join(home, name), "utf8");
    } catch {
      continue;
    }
    movedAside ||= name !== HOOK_FAILURES_FILE;
    kept.push(...parseFailures(text));
  }
  // Hooks take the time before they append, so lines from hooks running at once can be a little out of order.
  kept.sort((left, right) => left.at - right.at);
  const oldest = kept[0];
  const keptSince = movedAside && oldest !== undefined && oldest.at >= since ? oldest.at : null;
  return { failures: kept.filter(failure => failure.at >= since), keptSince };
}

function parseFailures(text: string): HookFailure[] {
  const failures: HookFailure[] = [];
  for (const line of text.split("\n")) {
    if (line.trim() === "") continue;
    try {
      const value = JSON.parse(line) as Partial<HookFailure>;
      if (typeof value.at === "number" && typeof value.event === "string" && typeof value.reason === "string") {
        failures.push({ at: value.at, host: value.host ?? "unknown", event: value.event, agent: value.agent ?? null, rerail: value.rerail ?? "unknown", reason: value.reason });
      }
    } catch {
      // A line cut short by a concurrent write.
    }
  }
  return failures;
}
