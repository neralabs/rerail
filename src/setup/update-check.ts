/**
 * The daemon's daily update check: once a day it asks npm for the version a bare `rerail update` would install, and
 * keeps the answer, so `/healthz` and the dashboard can say a newer Rerail is out. Unless a remote is added or Merge
 * Assist is on (internal testing), it is the only request Rerail makes on its own, apart from an npm rebuild of the
 * SQLite binding when it fails to load (docs/privacy.md). It installs nothing: `rerail update` stays the only way a
 * new version arrives.
 *
 * The answer and when the daemon last asked are kept in the Rerail home (`update-check.json`), so a daemon that
 * restarts asks again only once a day has passed, whether the last ask was answered or failed. A failed ask keeps the
 * last answer and logs one line; nothing about it reaches the dashboard. The daemon of a managed install runs it
 * (`cli/daemon-entry.ts`) unless `RERAIL_UPDATE_CHECK=0` is set for it; a checkout's daemon and tests never do.
 */
import { existsSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { z } from "zod";
import { ACTIVE_PROFILE_FILE } from "../activation/customer-profile.js";
import { replaceFileSync } from "../engine/file-replace.js";
import { UPDATE_CHECK_FILE, UPDATE_STATE_FILE } from "../engine/storage/rerail-home.js";
import { type NpmRunner, runNpm } from "../runtime/npm.js";
import { assertChannel, DEFAULT_UPDATE_CHANNEL, NpmPackageRegistry, type PackageRegistry } from "./updater.js";

export { UPDATE_CHECK_FILE };

/** How long an answer stands before the daemon asks again; a failed ask waits as long. */
export const UPDATE_CHECK_INTERVAL_MS = 24 * 60 * 60_000;
/** How often the daemon looks whether an ask is due: a computer asleep through the due time asks within this of waking. */
const DUE_LOOK_MS = 60 * 60_000;
/** The first look waits this long after the daemon starts, so a start never waits on npm. */
const FIRST_LOOK_MS = 60_000;

/** The newest Rerail on the channel and when npm said so (epoch ms), as `/healthz.update` reports it. */
export interface NewestRelease {
  latest: string;
  checkedAt: number;
}

/** A running update check: its last answer, and stopping it. */
export interface UpdateChecks {
  /**
   * The last answer, while its channel is still the one `rerail update` installs from; null before the first answer,
   * after a channel change until the next answer, and while a signed customer profile is active.
   */
  newest(): NewestRelease | null;
  /** Stops asking. An ask still out is not awaited: its answer is dropped, and npm ends on its own timeout. */
  stop(): void;
}

export interface UpdateCheckOptions {
  home: string;
  registry?: PackageRegistry;
  now?: () => number;
  intervalMs?: number;
  lookEveryMs?: number;
  firstLookMs?: number;
  log?: (line: string) => void;
}

const recordSchema = z.object({
  channel: z.string(),
  latest: z.string().nullable(),
  /** When npm last answered (epoch ms); null when it never has on this channel. */
  answeredAt: z.number().nullable(),
  /** When the daemon last asked, answered or not (epoch ms). */
  askedAt: z.number(),
}).strict();
type UpdateCheckRecord = z.infer<typeof recordSchema>;

/** Whether `RERAIL_UPDATE_CHECK` turns the check off: `0` or `false`, as `RERAIL_AUTOSTART` reads. */
export function updateCheckOff(env: NodeJS.ProcessEnv = process.env): boolean {
  return env.RERAIL_UPDATE_CHECK === "0" || env.RERAIL_UPDATE_CHECK === "false";
}

/**
 * Whether the daemon of `home` checks at all. A signed customer profile (internal testing) sets the channel
 * `rerail update` installs from; the daemon cannot read it without the device's keychain, so it does not check there.
 * A profile activated while the daemon runs stops the check too ({@link checkedChannel}).
 */
export function updateCheckApplies(home: string, env: NodeJS.ProcessEnv = process.env): boolean {
  return !updateCheckOff(env) && !profileActive(home);
}

function profileActive(home: string): boolean {
  return existsSync(join(home, ACTIVE_PROFILE_FILE));
}

/**
 * The channel the check asks about: the one a bare `rerail update` installs from, as `update-state.json` records it,
 * else `latest`. Undefined while a signed customer profile is active, whose channel the daemon cannot read.
 */
function checkedChannel(home: string): string | undefined {
  return profileActive(home) ? undefined : recordedChannel(home);
}

/** The channel a bare `rerail update` installs from: the one `update-state.json` records, else `latest`. */
function recordedChannel(home: string): string {
  try {
    const { channel } = JSON.parse(readFileSync(join(home, UPDATE_STATE_FILE), "utf8")) as { channel?: unknown };
    if (typeof channel !== "string") return DEFAULT_UPDATE_CHANNEL;
    assertChannel(channel);
    return channel;
  } catch {
    return DEFAULT_UPDATE_CHANNEL;
  }
}

function readRecord(path: string): UpdateCheckRecord | undefined {
  try {
    return recordSchema.parse(JSON.parse(readFileSync(path, "utf8")));
  } catch {
    return undefined;
  }
}

/** The answer `record` holds for `channel`, as `/healthz.update` reports it; null when it holds none for that channel. */
function answerFor(record: UpdateCheckRecord | undefined, channel: string | undefined): NewestRelease | null {
  if (record?.latest == null || record.answeredAt === null || record.channel !== channel) return null;
  return { latest: record.latest, checkedAt: record.answeredAt };
}

/**
 * npm run by the Node running the daemon, which finds the npm installed beside it first on PATH. A daemon autostarted by
 * an agent's hook inherits that agent's PATH, which may not reach npm at all (an app started from the macOS Dock gets
 * `/usr/bin:/bin:/usr/sbin:/sbin`).
 */
const npmBesideDaemonNode: NpmRunner = async (args, options) => await runNpm(args, { ...options, node: process.execPath });

/** Whether an ask is due: none yet on this channel, or the last one a day ago (or in the future, after a clock change). */
function askDue(record: UpdateCheckRecord | undefined, channel: string, now: number, intervalMs: number): boolean {
  if (record === undefined || record.channel !== channel) return true;
  return now - record.askedAt >= intervalMs || now < record.askedAt;
}

/** Starts the check for the daemon of `home`; the first look is `firstLookMs` after the start, then every `lookEveryMs`. */
export function startUpdateChecks(options: UpdateCheckOptions): UpdateChecks {
  const { home } = options;
  const registry = options.registry ?? new NpmPackageRegistry(npmBesideDaemonNode);
  const now = options.now ?? Date.now;
  const intervalMs = options.intervalMs ?? UPDATE_CHECK_INTERVAL_MS;
  const log = options.log ?? (line => process.stderr.write(line));
  const path = join(home, UPDATE_CHECK_FILE);
  let record = readRecord(path);
  let asking = false;
  let stopped = false;

  // A record of a few dozen bytes, written at most twice a day: written synchronously, two writes never interleave.
  const keep = (next: UpdateCheckRecord): void => {
    record = next;
    try {
      replaceFileSync(path, `${JSON.stringify(next, null, 2)}\n`, { mode: 0o600 });
    } catch (error) {
      log(`rerail daemon: could not keep the update check in ${path}: ${error instanceof Error ? error.message : String(error)}\n`);
    }
  };

  // The ask is kept before npm runs, so a daemon that restarts while npm is out does not ask again that day.
  const ask = async (channel: string): Promise<void> => {
    const askedAt = now();
    const kept = record?.channel === channel ? record : undefined;
    keep({ channel, latest: kept?.latest ?? null, answeredAt: kept?.answeredAt ?? null, askedAt });
    try {
      const latest = await registry.resolveVersion(channel);
      if (!stopped) keep({ channel, latest, answeredAt: now(), askedAt });
    } catch (error) {
      log(`rerail daemon: checking npm for a newer Rerail on ${channel} failed, asking again in a day: ${error instanceof Error ? error.message.split("\n")[0] : String(error)}\n`);
    }
  };

  const look = (): void => {
    if (stopped || asking) return;
    const channel = checkedChannel(home);
    if (channel === undefined || !askDue(record, channel, now(), intervalMs)) return;
    asking = true;
    void ask(channel).finally(() => { asking = false; });
  };

  const first = setTimeout(look, options.firstLookMs ?? FIRST_LOOK_MS);
  first.unref();
  const timer = setInterval(look, options.lookEveryMs ?? DUE_LOOK_MS);
  timer.unref();
  return {
    // Read on every `/healthz`: the channel can change, or a profile be activated, between two looks.
    newest: () => answerFor(record, checkedChannel(home)),
    stop: () => {
      stopped = true;
      clearTimeout(first);
      clearInterval(timer);
    },
  };
}
