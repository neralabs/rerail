import { homedir } from "node:os";
import { join, resolve } from "node:path";

/** The Rerail home: `$RERAIL_HOME`, or `~/.rerail`. Holds the database, daemon state and managed installs. */
export function defaultRerailHome(environment: NodeJS.ProcessEnv = process.env): string {
  return environment.RERAIL_HOME === undefined ? join(homedir(), ".rerail") : resolve(environment.RERAIL_HOME);
}

/** Internal testing: the remote sink's files in the Rerail home (runtime/remote-config.ts and remote-outbox.ts write them). */
export const REMOTE_CONFIG_FILE = "remote.json";
export const REMOTE_TOKENS_FOLDER = "remote-tokens";
export const REMOTE_OUTBOX_FILE = "remote-outbox.sqlite";
export const MACHINE_ID_FILE = "machine-id";
