import { homedir } from "node:os";
import { join, resolve } from "node:path";

/** The Rerail home: `$RERAIL_HOME`, or `~/.rerail`. Holds the database, daemon state and managed installs. */
export function defaultRerailHome(environment: NodeJS.ProcessEnv = process.env): string {
  return environment.RERAIL_HOME === undefined ? join(homedir(), ".rerail") : resolve(environment.RERAIL_HOME);
}
