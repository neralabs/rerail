import { spawnSync } from "node:child_process";
import { chmod, lstat, mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { keepFilesPrivate, makeRerailHome } from "../../../src/engine/storage/private-files.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
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
