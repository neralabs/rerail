import { homedir } from "node:os";
import { join, resolve } from "node:path";

/** The Rerail home: `$RERAIL_HOME`, or `~/.rerail`. Holds the database, daemon state and managed installs. */
export function defaultRerailHome(environment: NodeJS.ProcessEnv = process.env): string {
  return environment.RERAIL_HOME === undefined ? join(homedir(), ".rerail") : resolve(environment.RERAIL_HOME);
}

/** The one Rerail database: rules and every repository's history, in the Rerail home. */
export const RERAIL_DATABASE_FILE = "rerail.sqlite";
/** The daemon's files in the Rerail home: its state, its log, and the capability a loopback daemon keeps across restarts. */
export const DAEMON_STATE_FILE = "daemon.json";
export const DAEMON_LOG_FILE = "daemon.log";
export const DAEMON_ACCESS_TOKEN_FILE = "access-token";
/** In `~/.rerail` (never `RERAIL_HOME`): which Claude Code status lines Rerail registered. */
export const CLAUDE_STATUS_DATABASE_FILE = "harness.sqlite";

/** In the Rerail home: the updater's record of the installed versions and the update channel. */
export const UPDATE_STATE_FILE = "update-state.json";
/** In the Rerail home: the installed versions. */
export const VERSIONS_DIR = "versions";
/** In the Rerail home: the link to the active package root. */
export const CURRENT_LINK = "current";
/** In the Rerail home: the folder holding the launcher. */
export const LAUNCHER_DIR = "bin";

/** Internal testing: the remote sink's files in the Rerail home (runtime/remote-config.ts and remote-outbox.ts write them). */
export const REMOTE_CONFIG_FILE = "remote.json";
export const REMOTE_TOKENS_FOLDER = "remote-tokens";
export const REMOTE_OUTBOX_FILE = "remote-outbox.sqlite";
export const MACHINE_ID_FILE = "machine-id";
