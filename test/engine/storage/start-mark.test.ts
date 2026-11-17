import { execFileSync, spawn } from "node:child_process";
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { adoptRemote, linkAlias, pruneCheckouts, recordCheckout } from "../../../src/engine/catalog/repository-registry.js";
import {
  renewStartMark, START_MARK, startMarkFound, startMarkHolds, STOPPED_ANSWERS_DIR, STUCK_MARK,
} from "../../../src/engine/storage/start-mark.js";
import { setCheckoutStarted, setRepositoryStarted } from "../../../src/runtime/index.js";
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

/** A Rerail database in a home of its own under `root`. */
function database(root: string): RerailDatabase {
  const opened = openRerailDatabase(join(root, "home", "rerail.sqlite"));
  openDatabases.push(opened);
  return opened;
}

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
    const previous = process.cwd();
    process.chdir(root);
    try {
      renewStartMark({ $client: { name: ":memory:", memory: true } });
      renewStartMark({ $client: { name: "", memory: false } });
    } finally {
      process.chdir(previous);
    }
    expect(readdirSync(root)).toEqual([]);
  });

  it("writes and reads nothing through a link at the folder of kept answers, which can lead anywhere", async () => {
    const root = await temporary();
    const db = database(root);
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
    const db = database(root);
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

  it.runIf(process.platform === "win32")("writes the stuck mark where another program holds the mark open sharing nothing, so no kept mark holds", { timeout: 30_000 }, async () => {
    const root = await temporary();
    const db = database(root);
    renewStartMark(db);
    const found = markOf(db)!;
    const end = await heldSharingNothing(join(answers(db), START_MARK));
    try {
      renewStartMark(db);
      expect({ stuck: existsSync(join(answers(db), STUCK_MARK)), holds: startMarkHolds(db.$client.name, found) }).toEqual({ stuck: true, holds: false });
    } finally {
      await end();
    }
  });
});

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
