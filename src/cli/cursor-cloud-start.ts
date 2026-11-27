import { existsSync } from "node:fs";
import { appendFile, mkdir, readFile } from "node:fs/promises";
import { dirname, join, resolve } from "node:path";
import { gitText, runGit } from "../engine/git.js";
import { cursorCloudMarkerPath, markCursorCloud } from "../hosts/cursor/cursor-cloud.js";
import { defaultRerailHome } from "../engine/storage/rerail-home.js";
import { DAEMON_LOG_FILE, daemonEngineEndpoint, lookupDaemon, type DaemonState } from "../runtime/daemon-control.js";
import { CURSOR_HOOK_EVENTS, registerCursorProjectHooks, type RegistrationChange } from "../setup/harness-registration.js";
import type { RepositoryStartReport } from "./repository-setup.js";

/** The flag of `rerail start` that prepares a Cursor cloud agent's VM. */
export const CURSOR_CLOUD_FLAG = "--cursor-cloud";

/** The project hooks file a Cursor cloud agent loads, relative to the checkout's top folder, as Git names it. */
const PROJECT_HOOKS_FILE = ".cursor/hooks.json";

/** The daemon this VM's hooks send their events to, once it answers: where it serves, and its process. */
export type CloudDaemon = Pick<DaemonState, "url" | "pid">;

export interface CursorCloudStartOptions {
  /** The folder `rerail start` acts on: `--repo`, or the current folder. */
  workspace: string;
  /** The database `--database` names; the Rerail home's by default. The cloud marker goes beside it. */
  databasePath?: string;
  /** This install's package root, whose hook script the hook entries run (`registrationRoot`). */
  root: string;
  /** The node the hook entries run; the one set-up recorded by default. */
  nodePath?: string;
  platform?: NodeJS.Platform;
  say: (text: string) => void;
  /** Starts Rerail in the checkout, as a plain `rerail start` does (`startRerail`). */
  start: () => Promise<RepositoryStartReport>;
  /** Brings the daemon up, or finds it, and waits until it answers; tests pass their own. */
  daemon?: () => Promise<CloudDaemon>;
}

/** How the project hooks file is kept out of Git: listed in the exclude file, or tracked and skipped in this worktree. */
type KeptOutOfGit = { how: "excluded"; excludeFile: string; added: boolean } | { how: "skip-worktree" };

/**
 * `rerail start --cursor-cloud`: prepares a Cursor cloud agent's VM so the agent's hooks reach a running daemon
 * (docs/cursor-cloud-agents-prd.md, R1). Cursor's environment script runs it from the checkout, in `install` and again
 * in `start`, since a Build keeps the disk but no process. In order: it starts Rerail in the checkout as `rerail start`
 * does; writes Rerail's Cursor hooks into the checkout's `.cursor/hooks.json`, the only hooks a cloud agent loads; keeps
 * that file out of Git, so the agent's commits and pull requests never carry this machine's paths; marks the machine as
 * a cloud VM, which every Cursor hook reads; and waits for the daemon as a session's start does. It opens no session:
 * the agent starts its own, and Rerail records it from the agent's first event. Run again, it changes nothing.
 *
 * Where Rerail does not end up on in the checkout (a repository not set up, whose set-up stops at a question), it
 * throws, having written nothing for Cursor, so the environment script fails instead of booting an agent nothing checks.
 */
export async function startForCursorCloud(options: CursorCloudStartOptions): Promise<void> {
  const { say } = options;
  // Cloud VMs run Linux; the hook command written here is the POSIX one.
  if ((options.platform ?? process.platform) === "win32") {
    throw new Error(`${CURSOR_CLOUD_FLAG} is for Cursor cloud agents, whose VMs run Linux, and does not run on Windows. On this computer, run rerail start.`);
  }
  const report = await options.start();
  // Rerail is not on: in a cloud VM, set-up has not run, and the set-up this start ran stopped at a question.
  if (report.status !== "started") {
    throw new Error(`Rerail is not on in ${report.repositoryName}: the repository is not set up, and set-up stopped before it started Rerail, `
      + `so ${CURSOR_CLOUD_FLAG} wrote nothing for Cursor. Set the repository up first, in the install step of .cursor/environment.json: `
      + `rerail setup --agents cursor --repo "$PWD" --allow-sessions --answer pick-myself --packs <pack@version,...>. `
      + `Then run rerail start ${CURSOR_CLOUD_FLAG} again.`);
  }
  const checkout = checkoutTop(options.workspace);
  const hooks = await registerCursorProjectHooks({
    root: options.root, checkout, ...(options.nodePath === undefined ? {} : { nodePath: options.nodePath }),
  });
  const kept = await keepOutOfGit(checkout);
  await markCursorCloud(options.databasePath);
  const daemon = await (options.daemon ?? answeringDaemon)();
  say(`● Cursor cloud agents in ${report.repositoryName} reach Rerail now.`);
  say(`  Hooks: ${hooks.path}, for ${CURSOR_HOOK_EVENTS.length} Cursor events (${hooksAction(hooks)}).`);
  say(`  Kept out of Git: ${keptText(kept)}.`);
  say(`  Cloud marker: ${cursorCloudMarkerPath(options.databasePath)}.`);
  say(`  Daemon: running at ${daemon.url} (pid ${daemon.pid}). The agent starts its own session; Rerail records it from the agent's hooks.`);
}

/** The top folder of the checkout `workspace` is in: Cursor runs project hooks from there and reads its `.cursor/`. */
function checkoutTop(workspace: string): string {
  const top = gitText(workspace, ["rev-parse", "--show-toplevel"]);
  if (top === undefined || top === "") throw new Error(`${workspace} is not inside a Git checkout, so there is no project for Cursor's hooks.`);
  return resolve(top);
}

/**
 * Keeps the project hooks file out of Git, so the agent never commits this machine's hook entries. A file the
 * repository does not track is listed in the checkout's exclude file, which Git reads for every worktree of the clone
 * and never commits (`--git-path`, so a linked worktree finds the clone's), once. A file it tracks, because the
 * repository commits hooks of its own, is marked skip-worktree in this checkout's index, so the entries added here
 * show in no diff; marking it again changes nothing.
 */
async function keepOutOfGit(checkout: string): Promise<KeptOutOfGit> {
  if (runGit(checkout, ["ls-files", "--error-unmatch", "--", PROJECT_HOOKS_FILE]).status === 0) {
    const marked = runGit(checkout, ["update-index", "--skip-worktree", "--", PROJECT_HOOKS_FILE]);
    if (marked.status !== 0) throw new Error(`Git could not skip ${PROJECT_HOOKS_FILE} in ${checkout}: ${marked.stderr.trim() || marked.error?.message}`);
    return { how: "skip-worktree" };
  }
  const listed = gitText(checkout, ["rev-parse", "--git-path", "info/exclude"]);
  if (listed === undefined || listed === "") throw new Error(`Git did not name the exclude file of ${checkout}.`);
  const excludeFile = resolve(checkout, listed);
  const text = existsSync(excludeFile) ? await readFile(excludeFile, "utf8") : "";
  if (text.split(/\r?\n/u).some(line => [PROJECT_HOOKS_FILE, `/${PROJECT_HOOKS_FILE}`].includes(line.trim()))) {
    return { how: "excluded", excludeFile, added: false };
  }
  await mkdir(dirname(excludeFile), { recursive: true });
  await appendFile(excludeFile, `${text === "" || text.endsWith("\n") ? "" : "\n"}${PROJECT_HOOKS_FILE}\n`);
  return { how: "excluded", excludeFile, added: true };
}

/**
 * The daemon, started or found as the hook of a session's start finds it (`daemonEngineEndpoint`): this package's in
 * place of another package's, a busy or starting one waited for, and none started under `RERAIL_AUTOSTART=0`, which
 * then throws. It opens no session. Then where it serves, for the summary.
 */
async function answeringDaemon(): Promise<CloudDaemon> {
  await daemonEngineEndpoint(true);
  const lookup = await lookupDaemon(defaultRerailHome());
  if (!("state" in lookup)) throw new Error(`The Rerail daemon answered, then stopped. Run rerail start ${CURSOR_CLOUD_FLAG} again, or see ${join(defaultRerailHome(), DAEMON_LOG_FILE)}.`);
  return lookup.state;
}

function hooksAction(change: RegistrationChange): string {
  if (change.action === "created") return "written";
  return change.action === "updated" ? "Rerail's entries added" : "already in place";
}

function keptText(kept: KeptOutOfGit): string {
  if (kept.how === "skip-worktree") return `the repository tracks ${PROJECT_HOOKS_FILE}, so this checkout skips it (git update-index --skip-worktree)`;
  return `${PROJECT_HOOKS_FILE} is listed in ${kept.excludeFile}${kept.added ? "" : " already"}`;
}
