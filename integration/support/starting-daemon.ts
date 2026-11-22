/**
 * A Rerail home of its own whose daemon a test holds in its start (`daemon-start-gate.mjs`), and the built CLI run in
 * it as a person runs it: autostart on, its own user home, no browser. Every daemon process started in it notes its pid,
 * so a test counts the daemons that run once its commands end.
 */
import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readdirSync, realpathSync, rmSync, symlinkSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { text } from "node:stream/consumers";
import { fileURLToPath, pathToFileURL } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const PACKAGE_ROOT = resolve(HERE, "..", "..");
const CLI = join(PACKAGE_ROOT, "dist", "cli", "cli.js");
const GATE = pathToFileURL(join(HERE, "daemon-start-gate.mjs")).href;

/** How long a test waits for a fact it is owed (a held daemon, a command's end) before it says which one never came. */
const FACT_BOUND_MS = 90_000;

export interface Ran {
  code: number;
  stdout: string;
  stderr: string;
}

export interface Command {
  /** What the command printed and how it exited, once it has. */
  done: Promise<Ran>;
  /** Whether it has exited. */
  ended: () => boolean;
  /** What it has written to stderr so far. */
  stderr: () => string;
}

/** Waits until `fact()` holds, looking every 20 ms; throws naming `what` if it has not within {@link FACT_BOUND_MS}. */
export async function until(fact: () => boolean, what: string): Promise<void> {
  const bound = Date.now() + FACT_BOUND_MS;
  while (!fact()) {
    if (Date.now() > bound) throw new Error(`${what} did not happen within ${FACT_BOUND_MS / 1_000}s`);
    await new Promise(done => setTimeout(done, 20));
  }
}

function alive(pid: number): boolean {
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return (error as NodeJS.ErrnoException).code === "EPERM";
  }
}

export class StartingDaemonHome {
  private constructor(
    readonly root: string,
    readonly home: string,
    readonly repo: string,
    private readonly gate: string,
    private readonly env: NodeJS.ProcessEnv,
  ) {}

  /** A home past set-up's one sessions approval, with this build active, and a Git checkout to run set-up in. */
  static make(): StartingDaemonHome {
    const root = realpathSync(mkdtempSync(join(tmpdir(), "rerail-starting-daemon-")));
    const home = join(root, "rerail-home");
    const userHome = join(root, "user-home");
    const repo = join(root, "repo");
    const gate = join(root, "gate");
    for (const folder of [home, userHome, repo, gate]) mkdirSync(folder);
    symlinkSync(PACKAGE_ROOT, join(home, "current"), "junction");
    writeFileSync(join(home, "consent.json"), JSON.stringify({ sessions: { approvedAt: "2026-11-01T00:00:00.000Z" } }));
    writeFileSync(join(repo, "a.txt"), "a\n");
    execFileSync("git", ["init", "-q"], { cwd: repo });
    const env: NodeJS.ProcessEnv = {
      ...process.env,
      RERAIL_HOME: home,
      HOME: userHome,
      USERPROFILE: userHome,
      BROWSER: "none",
      NODE_OPTIONS: `${process.env.NODE_OPTIONS ?? ""} --import=${JSON.stringify(GATE)}`.trim(),
      RERAIL_TEST_DAEMON_START_GATE: gate,
    };
    // A person's shell has autostart on; the test run's own setting keeps every other test from starting a daemon.
    delete env.RERAIL_AUTOSTART;
    return new StartingDaemonHome(root, home, repo, gate, env);
  }

  /** `rerail <args>` from the built CLI, in the checkout. */
  run(args: readonly string[]): Command {
    const child = spawn(process.execPath, [CLI, ...args], { cwd: this.repo, env: this.env, stdio: ["ignore", "pipe", "pipe"] });
    let stderr = "";
    let ended = false;
    child.stderr.setEncoding("utf8");
    child.stderr.on("data", (chunk: string) => {
      stderr += chunk;
    });
    const stdout = text(child.stdout);
    const done = new Promise<number>((finish, fail) => {
      child.once("error", fail);
      child.once("close", code => finish(code ?? 1));
    }).then(async code => {
      ended = true;
      return { code, stdout: await stdout, stderr };
    });
    return { done, ended: () => ended, stderr: () => stderr };
  }

  /** The pid of the first daemon held in its start, once one is. */
  async held(): Promise<number> {
    const found = (): string | undefined => readdirSync(this.gate).find(name => name.startsWith("held-"));
    await until(() => found() !== undefined, "a daemon held in its start");
    return Number(found()!.slice("held-".length));
  }

  /** Whether the test has let the held daemon go on. */
  released(): boolean {
    return existsSync(join(this.gate, "go"));
  }

  /** Lets the held daemon go on with its start. */
  release(): void {
    writeFileSync(join(this.gate, "go"), "");
  }

  /**
   * Lets the held daemon go on once one of `commands` says on stderr that it waits for it (`waiting`), as a command does
   * for a daemon another process launched, or once one has ended: a command that gave up beside a daemon still starting
   * has ended while it was held.
   */
  async releaseOnceWaitingOrEnded(commands: readonly Command[], waiting: RegExp): Promise<void> {
    await until(() => commands.some(command => command.ended() || waiting.test(command.stderr())), "a command waiting, or one ending");
    this.release();
  }

  /** The daemons started in this home that run now. */
  daemons(): number[] {
    return readdirSync(this.gate).filter(name => name.startsWith("daemon-")).map(name => Number(name.slice("daemon-".length))).filter(alive);
  }

  /** Stops the home's daemon, ends any other daemon it started, and removes the folder. */
  async end(): Promise<void> {
    if (!this.released()) this.release();
    await this.run(["daemon", "stop"]).done;
    for (const pid of this.daemons()) process.kill(pid);
    await until(() => this.daemons().length === 0, "every daemon of the home ending");
    rmSync(this.root, { recursive: true, force: true });
  }
}

/** What a start says while it waits for a daemon still in its start that another process launched. */
export const WAITS_BESIDE_STARTING = /is still starting; this start waits for it to answer/u;

export interface TwoCommandsSeen {
  setup: Ran;
  start: Ran;
  /** Whether `rerail daemon start` began while set-up's daemon was still held in its start. */
  startBesideHeld: boolean;
  daemons: number;
}

/**
 * `rerail setup --answer start`, then `rerail daemon start`, as a script runs them, while the daemon set-up starts is
 * held in its start: until set-up says it waits for it, or until set-up has ended; then until the start says it waits,
 * or has ended.
 */
export async function setupThenStart(place: StartingDaemonHome): Promise<TwoCommandsSeen> {
  const setup = place.run(["setup", "--answer", "start"]);
  await place.held();
  await until(() => setup.ended() || WAITS_BESIDE_STARTING.test(setup.stderr()), "set-up waiting, or ending");
  if (!setup.ended()) place.release();
  const setupRan = await setup.done;
  const startBesideHeld = !place.released();
  const start = place.run(["daemon", "start"]);
  if (startBesideHeld) await place.releaseOnceWaitingOrEnded([start], WAITS_BESIDE_STARTING);
  return { setup: setupRan, start: await start.done, startBesideHeld, daemons: place.daemons().length };
}

export interface TwoStartsSeen {
  starts: Ran[];
  daemons: number;
}

/** Two `rerail daemon start` at once in a home with no daemon, the daemon one of them launches held in its start. */
export async function twoStartsAtOnce(place: StartingDaemonHome): Promise<TwoStartsSeen> {
  const starts = [place.run(["daemon", "start"]), place.run(["daemon", "start"])];
  await place.held();
  await place.releaseOnceWaitingOrEnded(starts, WAITS_BESIDE_STARTING);
  return { starts: await Promise.all(starts.map(async start => await start.done)), daemons: place.daemons().length };
}
