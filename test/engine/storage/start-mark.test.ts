import { execFileSync, spawn } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { performance } from "node:perf_hooks";
import { afterEach, describe, expect, it, vi } from "vitest";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { adoptRemote, linkAlias, pruneCheckouts, recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import {
  renewStartMark, START_MARK, startMarkFound, startMarkHolds, STOPPED_ANSWERS_DIR, STUCK_MARK,
} from "../../../src/engine/storage/start-mark.js";
import { makeAnswersFolder, setCheckoutStarted, setRepositoryStarted } from "../../../src/runtime/index.js";
import { linkFolder, symlinkOrSkip } from "../../support/platform/links.js";
import { NO_POSIX_MODES, POSIX_MODES } from "../../support/platform/file-mode.js";
import { removeFolder, removeFolderSync } from "../../support/platform/remove-folder.js";

const temporaryDirectories: string[] = [];
const openDatabases: RerailDatabase[] = [];
afterEach(async () => {
  for (const database of openDatabases.splice(0)) database.$client.close();
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function temporary(): Promise<string> {
  const path = realpathSync(await mkdtemp(join(tmpdir(), "rerail-start-mark-")));
  temporaryDirectories.push(path);
  return path;
}

/** A Rerail database in a home of its own under `root`, where a daemon has made the folder of kept answers at its start. */
function database(root: string): RerailDatabase {
  const opened = homeNoDaemonRan(root);
  makeAnswersFolder(join(root, "home"));
  return opened;
}

/** A Rerail database in a home of its own under `root` where no daemon has run, so no folder of kept answers is there. */
function homeNoDaemonRan(root: string): RerailDatabase {
  const opened = openRerailDatabase(join(root, "home", "rerail.sqlite"));
  openDatabases.push(opened);
  return opened;
}

/** Each entry in the folder at `path` and below it, sorted. */
const listing = (path: string): string[] => readdirSync(path, { recursive: true }).map(String).sort();

function git(cwd: string, ...args: string[]): void {
  execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@example.invalid", ...args], { cwd, stdio: "ignore" });
}

/** A Git repository at `path` with one commit, and no remote. */
function gitRepository(path: string): string {
  mkdirSync(path, { recursive: true });
  git(path, "init", "-q", "-b", "main");
  git(path, "commit", "-q", "--allow-empty", "-m", "base");
  return path;
}

const markOf = (db: RerailDatabase): string | undefined => startMarkFound(db.$client.name);
const answers = (db: RerailDatabase): string => join(dirname(db.$client.name), STOPPED_ANSWERS_DIR);

/** Each change to where Rerail runs, made on a database where `app`, a Git checkout with no remote, is recorded. */
const writers: Array<{ what: string; write: (db: RerailDatabase, app: string, root: string) => void }> = [
  { what: "a repository started or stopped", write: (db, app) => setRepositoryStarted(db, recordCheckout(db, app).repositoryKey, false) },
  { what: "a checkout started or stopped", write: (db, app) => setCheckoutStarted(db, recordCheckout(db, app), false) },
  {
    what: "a checkout recorded again for another clone", write: (db, app) => {
      removeFolderSync(join(app, ".git"));
      gitRepository(app);
      recordCheckout(db, app);
    },
  },
  { what: "a repository that takes a remote", write: (db, app) => adoptRemote(db, recordCheckout(db, app).repositoryKey, "remote:github.com/acme/web") },
  { what: "a repository that takes an alias", write: (db, app) => linkAlias(db, recordCheckout(db, app).repositoryKey, "https://github.com/acme/old-web") },
  {
    what: "checkouts forgotten", write: (db, _app, root) => {
      const gone = gitRepository(join(root, "gone"));
      recordCheckout(db, gone);
      removeFolderSync(gone);
      expect(pruneCheckouts(db)).toHaveLength(1);
    },
  },
];

describe("the start mark", () => {
  it.each(writers)("is renewed beside the database once $what commits", async ({ write }) => {
    const root = await temporary();
    const db = database(root);
    const app = gitRepository(join(root, "app"));
    recordCheckout(db, app);
    const before = markOf(db);
    write(db, app, root);
    const after = markOf(db);
    expect({ before, after: after?.split(":").length, renewed: after !== before }).toEqual({ before: "none", after: 4, renewed: true });
  });

  it.each(writers)("writes nothing in a home where no daemon made the folder of kept answers once $what commits, so no hook keeps an answer there", async ({ write }) => {
    const root = await temporary();
    const db = homeNoDaemonRan(root);
    const app = gitRepository(join(root, "app"));
    recordCheckout(db, app);
    const before = listing(join(root, "home"));
    write(db, app, root);
    expect({ home: listing(join(root, "home")), mark: markOf(db) }).toEqual({ home: before, mark: undefined });
  });

  it("is a new file at each renewal, in a folder only this user may open and a file only this user may read and write", async ({ skip }) => {
    const root = await temporary();
    const db = database(root);
    renewStartMark(db);
    const first = markOf(db);
    renewStartMark(db);
    expect(markOf(db)).not.toBe(first);
    if (!POSIX_MODES) skip(NO_POSIX_MODES);
    expect([answers(db), join(answers(db), START_MARK)].map(path => (statSync(path).mode & 0o777).toString(8))).toEqual(["700", "600"]);
  });

  it("writes nothing for a database in memory", async () => {
    const root = await temporary();
    // A folder of kept answers where a database with no file of its own would find one, so a renewal there shows.
    mkdirSync(join(root, STOPPED_ANSWERS_DIR));
    const previous = process.cwd();
    process.chdir(root);
    try {
      renewStartMark({ $client: { name: ":memory:", memory: true } });
      renewStartMark({ $client: { name: "", memory: false } });
    } finally {
      process.chdir(previous);
    }
    expect(listing(root)).toEqual([STOPPED_ANSWERS_DIR]);
  });

  it("writes and reads nothing through a link at the folder of kept answers, which can lead anywhere", async () => {
    const root = await temporary();
    const db = homeNoDaemonRan(root);
    const outside = join(root, "outside");
    mkdirSync(outside);
    writeFileSync(join(outside, START_MARK), "1\n");
    linkFolder(outside, answers(db));
    renewStartMark(db);
    expect({ outside: readdirSync(outside), mark: readFileSync(join(outside, START_MARK), "utf8"), found: markOf(db) })
      .toEqual({ outside: [START_MARK], mark: "1\n", found: undefined });
  });

  it("reads no mark through a link at its name, nor a folder there", async ctx => {
    const root = await temporary();
    const db = homeNoDaemonRan(root);
    const outside = join(root, "outside");
    mkdirSync(outside);
    writeFileSync(join(outside, START_MARK), "1\n");
    mkdirSync(answers(db));
    symlinkOrSkip(ctx, join(outside, START_MARK), join(answers(db), START_MARK), "file");
    expect(markOf(db)).toBeUndefined();
    rmSync(join(answers(db), START_MARK));
    mkdirSync(join(answers(db), START_MARK));
    expect(markOf(db)).toBeUndefined();
  });

  it("is none where nothing is there yet, and holds only while it is the one found and no renewal of it has failed since", async () => {
    const root = await temporary();
    const db = database(root);
    expect(markOf(db)).toBe("none");
    expect(startMarkHolds(db.$client.name, "none")).toBe(true);
    renewStartMark(db);
    const found = markOf(db)!;
    expect({ none: startMarkHolds(db.$client.name, "none"), found: startMarkHolds(db.$client.name, found) }).toEqual({ none: false, found: true });
    writeFileSync(join(answers(db), STUCK_MARK), "");
    expect(startMarkHolds(db.$client.name, found)).toBe(false);
  });

  it("is not found, and no mark holds, where the folder of kept answers is not there, since no renewal is recorded then", async () => {
    const root = await temporary();
    const db = homeNoDaemonRan(root);
    expect({ found: markOf(db), none: startMarkHolds(db.$client.name, "none") }).toEqual({ found: undefined, none: false });
  });

  it("writes the stuck mark where a renewal cannot replace the mark, and the next renewal that can removes it", async () => {
    const root = await temporary();
    const db = database(root);
    renewStartMark(db);
    rmSync(join(answers(db), START_MARK));
    // A folder with a file in it at the mark's name: no rename replaces it.
    mkdirSync(join(answers(db), START_MARK, "held"), { recursive: true });
    renewStartMark(db);
    expect(existsSync(join(answers(db), STUCK_MARK))).toBe(true);
    removeFolderSync(join(answers(db), START_MARK));
    renewStartMark(db);
    const found = markOf(db)!;
    expect({ stuck: existsSync(join(answers(db), STUCK_MARK)), holds: startMarkHolds(db.$client.name, found) }).toEqual({ stuck: false, holds: true });
  });

  it.runIf(process.platform === "win32")("writes the stuck mark at once where another program holds the mark open sharing nothing, so no kept mark holds, and renews the mark once the program lets it go", { timeout: 30_000 }, async () => {
    const root = await temporary();
    const db = database(root);
    renewStartMark(db);
    const found = markOf(db)!;
    const end = await heldSharingNothing(join(answers(db), START_MARK));
    let took: number;
    try {
      const begun = performance.now();
      renewStartMark(db);
      took = performance.now() - begun;
      expect({ stuck: existsSync(join(answers(db), STUCK_MARK)), holds: startMarkHolds(db.$client.name, found) }).toEqual({ stuck: true, holds: false });
    } finally {
      await end();
    }
    // A wait here holds the writer, and where the writer is the daemon every hook asking it: 2.2 s before.
    expect(took).toBeLessThan(500);
    await vi.waitFor(() => expect(existsSync(join(answers(db), STUCK_MARK))).toBe(false), { timeout: 5_000, interval: 50 });
    expect({ renewed: markOf(db) !== found, holds: startMarkHolds(db.$client.name, markOf(db)!) }).toEqual({ renewed: true, holds: true });
  });

  it("tries a renewal it could not make again later, without waiting for it, and the stuck mark goes once one succeeds", async () => {
    const root = await temporary();
    const db = database(root);
    renewStartMark(db);
    rmSync(join(answers(db), START_MARK));
    // A folder with a file in it at the mark's name: no rename replaces it, and none waits for it.
    mkdirSync(join(answers(db), START_MARK, "held"), { recursive: true });
    renewStartMark(db);
    expect(existsSync(join(answers(db), STUCK_MARK))).toBe(true);
    removeFolderSync(join(answers(db), START_MARK));
    await vi.waitFor(() => expect(existsSync(join(answers(db), STUCK_MARK))).toBe(false), { timeout: 5_000, interval: 50 });
    expect(startMarkHolds(db.$client.name, markOf(db)!)).toBe(true);
  });

  it("does not hold where the folder takes no new file, so that neither a renewal nor its stuck mark can be written", async ({ skip }) => {
    if (!POSIX_MODES) skip(NO_POSIX_MODES);
    if (process.getuid?.() === 0) skip("root may add files to any folder");
    const root = await temporary();
    const db = database(root);
    renewStartMark(db);
    const found = markOf(db)!;
    chmodSync(answers(db), 0o500);
    try {
      renewStartMark(db);
      expect({ mark: markOf(db), stuck: existsSync(join(answers(db), STUCK_MARK)), holds: startMarkHolds(db.$client.name, found) })
        .toEqual({ mark: found, stuck: false, holds: false });
    } finally {
      chmodSync(answers(db), 0o700);
    }
    expect(startMarkHolds(db.$client.name, found)).toBe(true);
  });

  it.each(writers)("is renewed only once $what has committed, never inside its transaction", async ({ write }) => {
    const root = await temporary();
    const db = database(root);
    const app = gitRepository(join(root, "app"));
    recordCheckout(db, app);
    const watched = transactionAtEachRenewal(db);
    write(watched.db, app, root);
    expect({ renewed: watched.inTransaction.length > 0, inside: watched.inTransaction.filter(Boolean).length }).toEqual({ renewed: true, inside: 0 });
  });
});

/**
 * `db` with each look at its connection's file name, which a renewal makes once before it writes anything, recording
 * whether a transaction is open on it then.
 */
function transactionAtEachRenewal(db: RerailDatabase): { db: RerailDatabase; inTransaction: boolean[] } {
  const inTransaction: boolean[] = [];
  const client = new Proxy(db.$client, {
    get(target, property) {
      if (property === "name") inTransaction.push(target.inTransaction);
      const value: unknown = Reflect.get(target, property, target);
      return typeof value === "function" ? (value as (...args: unknown[]) => unknown).bind(target) : value;
    },
  });
  return { db: new Proxy(db, { get: (target, property, receiver) => (property === "$client" ? client : Reflect.get(target, property, receiver)) }), inTransaction };
}

/** Another program holding the file at `path` open and sharing nothing, until the function it gives ends it. */
async function heldSharingNothing(path: string): Promise<() => Promise<void>> {
  const command = `$f = [System.IO.File]::Open('${path.replaceAll("'", "''")}', 'Open', 'Read', 'None'); `
    + "[Console]::Out.WriteLine('held'); [Console]::Out.Flush(); Start-Sleep -Seconds 600";
  const powershell = join(process.env.SystemRoot ?? String.raw`C:\Windows`, "System32", "WindowsPowerShell", "v1.0", "powershell.exe");
  const child = spawn(powershell, ["-NoProfile", "-NonInteractive", "-Command", command], { stdio: ["ignore", "pipe", "ignore"], windowsHide: true });
  const exited = new Promise<void>(resolve => child.once("exit", () => resolve()));
  const end = async (): Promise<void> => {
    if (child.exitCode === null) child.kill();
    await exited;
  };
  let said = "";
  const held = new Promise<void>((resolve, reject) => {
    child.stdout.on("data", (chunk: Buffer) => {
      said += chunk.toString();
      if (said.includes("held")) resolve();
    });
    child.once("exit", () => reject(new Error(`the holder ended before it held ${path}: ${said}`)));
  });
  try {
    await held;
  } catch (error) {
    await end();
    throw error;
  }
  return end;
}
