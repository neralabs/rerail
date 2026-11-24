import { spawnSync } from "node:child_process";
import { chmodSync } from "node:fs";
import { chmod, lstat, mkdir, mkdtemp, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { makeLedgerFolder } from "../../../src/engine/storage/ledger-paths.js";
import { keepFilesPrivate, keepFolderPrivate, makeRerailHome, openToOthers } from "../../../src/engine/storage/private-files.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

// Spied, each call still the real one, so a case can make `chmod` fail as a system that refuses it does.
vi.mock("node:fs", { spy: true });

const temporaryDirectories: string[] = [];

afterEach(async () => {
  vi.restoreAllMocks();
  vi.mocked(chmodSync).mockReset();
  await Promise.all(temporaryDirectories.splice(0).map(path => removeFolder(path)));
});

async function temporaryDirectory(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-private-files-"));
  temporaryDirectories.push(root);
  return root;
}

async function mode(path: string): Promise<number> {
  return (await lstat(path)).mode & 0o777;
}

describe.skipIf(process.platform === "win32")("what Rerail keeps private", () => {
  it("is a new Rerail home, made readable by its owner only", async () => {
    const home = join(await temporaryDirectory(), "home");
    makeRerailHome(home);
    expect(await mode(home)).toBe(0o700);
  });

  it("is the folder of the events hooks left while the daemon did not read, kept private as the home is made", async () => {
    const home = join(await temporaryDirectory(), "home");
    await mkdir(join(home, "untaken-events"), { recursive: true });
    await chmod(join(home, "untaken-events"), 0o755);
    makeRerailHome(home);
    expect(await mode(join(home, "untaken-events"))).toBe(0o700);
  });

  it("is a folder kept private on every call, made or not, since a chmod starts no program", async () => {
    const folder = join(await temporaryDirectory(), "tokens");
    await mkdir(folder);
    await chmod(folder, 0o755);
    keepFolderPrivate(folder);
    expect(await mode(folder)).toBe(0o700);
  });

  it("takes only the list from a session records folder another program made, so an account it let in still makes its own folder there", async () => {
    const root = await temporaryDirectory();
    // 2775: a folder a person shares with their group (setgid, group-writable); 777 and 755, as Git or `mkdir` leave one
    // under a loose umask; 700, as Rerail makes one. One the group or others may still add to also becomes sticky.
    for (const [found, kept] of [[0o2775, 0o3731], [0o777, 0o1733], [0o755, 0o711], [0o700, 0o700]] as const) {
      const sessions = join(root, found.toString(8), ".rerail", "sessions");
      await mkdir(sessions, { recursive: true });
      await chmod(sessions, found);
      // macOS drops the set-group-ID bit for an account outside the folder's group (wheel, in `/tmp`); it stays dropped.
      const dropped = found & ~((await lstat(sessions)).mode & 0o7777) & 0o7000;
      const session = join(sessions, "a".repeat(64));
      await makeLedgerFolder(session, { repair: true });
      expect((await lstat(sessions)).mode & 0o7777, found.toString(8)).toBe(kept & ~dropped);
      expect(await mode(session)).toBe(0o700);
    }
  });

  it("opens every session in a records folder whose mode change the system refuses, says so once, and tries again once its mode changed", async () => {
    const sessions = join(await temporaryDirectory(), ".rerail", "sessions");
    await mkdir(sessions, { recursive: true });
    await chmod(sessions, 0o755);
    const written = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    vi.mocked(chmodSync).mockImplementation(path => {
      throw Object.assign(new Error(`EPERM: operation not permitted, chmod '${String(path)}'`), { code: "EPERM" });
    });
    const tried = (): number => vi.mocked(chmodSync).mock.calls.filter(([path]) => path === sessions).length;
    const named = (): number => written.mock.calls.filter(([line]) => String(line).startsWith(`rerail: could not keep ${sessions} `)).length;
    const open = async (id: string): Promise<void> => {
      // What the daemon runs as it opens a session (`#openLedger`).
      const session = join(sessions, id.repeat(64));
      await makeLedgerFolder(session, { repair: true });
      keepFolderPrivate(session);
      expect(await mode(session)).toBe(0o700);
      await new Promise(resolve => setImmediate(resolve));
    };
    await open("a");
    await open("b");
    expect([tried(), named()]).toEqual([1, 1]);
    // Its mode changed, then another account's session made its folder there, which sets the change time again.
    await chmod(sessions, 0o750);
    await mkdir(join(sessions, "e".repeat(64)));
    await open("c");
    expect([tried(), named()]).toEqual([2, 2]);
  });

  it("is a secret with no group or other permission, and one with any is named with its mode", async () => {
    const key = join(await temporaryDirectory(), "key.pem");
    await writeFile(key, "key");
    await chmod(key, 0o600);
    expect(await openToOthers(key)).toBeUndefined();
    await chmod(key, 0o620);
    expect(await openToOthers(key)).toBe("is open to other users (mode 0620)");
    await expect(openToOthers(join(key, "..", "missing.pem"))).rejects.toThrow(/ENOENT/u);
  });

  it("takes group and other access from an existing Rerail home and the private files in it", async () => {
    const home = join(await temporaryDirectory(), "home");
    await mkdir(home, { mode: 0o755 });
    await chmod(home, 0o755);
    const privateFiles = ["rerail.sqlite.pre-update", "rerail.sqlite-wal", "harness.sqlite", "daemon.json", "access-token", "daemon.log"];
    for (const name of [...privateFiles, "notes.txt"]) {
      await writeFile(join(home, name), "x");
      await chmod(join(home, name), 0o644);
    }
    makeRerailHome(home);
    expect(await mode(home)).toBe(0o700);
    for (const name of privateFiles) expect(await mode(join(home, name)), name).toBe(0o600);
    // Not Rerail's to change.
    expect(await mode(join(home, "notes.txt"))).toBe(0o644);
  });

  it("follows no symlink in the Rerail home", async () => {
    const root = await temporaryDirectory();
    const home = join(root, "home");
    const outside = join(root, "outside.json");
    await mkdir(home, { mode: 0o700 });
    await writeFile(outside, "{}");
    await chmod(outside, 0o644);
    await symlink(outside, join(home, "daemon.json"));
    makeRerailHome(home);
    expect(await mode(outside)).toBe(0o644);
  });

  it("takes group and other access from a Rerail database a writer opens, and from the files named after it", async () => {
    const path = join(await temporaryDirectory(), "home", "rerail.sqlite");
    openRerailDatabase(path).$client.close();
    expect(await mode(path)).toBe(0o600);
    await chmod(path, 0o644);
    await writeFile(`${path}.pre-update`, "copy");
    await chmod(`${path}.pre-update`, 0o644);
    openRerailDatabase(path).$client.close();
    expect(await mode(path)).toBe(0o600);
    expect(await mode(`${path}.pre-update`)).toBe(0o600);
  });

  it("keeps the locks a connection in this process holds while it tightens that database", async () => {
    const path = join(await temporaryDirectory(), "rerail.sqlite");
    const holder = new Database(path);
    try {
      holder.exec("CREATE TABLE t (x); BEGIN IMMEDIATE; INSERT INTO t VALUES (1);");
      await chmod(path, 0o644);
      keepFilesPrivate(join(path, ".."), ["rerail.sqlite"]);
      expect(await mode(path)).toBe(0o600);
      // Another process must still find the database locked: closing a descriptor of the file would have dropped
      // this process's POSIX locks, and with them the write transaction above.
      const writer = spawnSync(process.execPath, ["-e", `
        const Database = require(${JSON.stringify(require.resolve("better-sqlite3"))});
        const db = new Database(${JSON.stringify(path)}, { timeout: 0 });
        try { db.exec("BEGIN IMMEDIATE"); process.stdout.write("acquired"); } catch (error) { process.stdout.write(error.code); }
      `], { encoding: "utf8" });
      expect(writer.stdout).toBe("SQLITE_BUSY");
    } finally {
      holder.close();
    }
  });
});
