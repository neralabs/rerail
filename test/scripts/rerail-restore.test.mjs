import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, readlinkSync, rmSync, statSync, writeFileSync } from "node:fs";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { test } from "vitest";

import {
  installIntoHome,
  loadSqlite,
  parseRestoreArguments,
  processAlive,
  restartArguments,
  restoreBackup,
  restoreInstalled,
  selectBackup,
  stageDatabase,
  stopDaemon,
} from "../../scripts/dev/rerail-restore.mjs";

const { DatabaseSync } = await loadSqlite();

async function withHome(run) {
  const home = mkdtempSync(join(tmpdir(), "rerail-restore-test-"));
  try {
    return await run(home);
  } finally {
    rmSync(home, { recursive: true, force: true });
  }
}

function envFor(home) {
  return { HOME: home, PATH: process.env.PATH ?? "" };
}

/** Every file and symlink under `root` with its contents or target, for exact before/after comparison. */
function snapshot(root) {
  const entries = {};
  const walk = directory => {
    for (const entry of readdirSync(directory, { withFileTypes: true })) {
      const path = join(directory, entry.name);
      if (entry.isSymbolicLink()) entries[path] = `-> ${readlinkSync(path)}`;
      else if (entry.isDirectory()) walk(path);
      else entries[path] = `${statSync(path).mode}:${readFileSync(path, "base64")}`;
    }
  };
  walk(root);
  return entries;
}

/** A WAL database whose rows live only in its -wal: the main file alone would read as empty. */
function walOnlyDatabase(directory, name, rows) {
  mkdirSync(directory, { recursive: true });
  const path = join(directory, name);
  const writer = new DatabaseSync(path);
  writer.exec("PRAGMA journal_mode = WAL; PRAGMA wal_autocheckpoint = 0; CREATE TABLE catalog_rules (id text PRIMARY KEY)");
  const insert = writer.prepare("INSERT INTO catalog_rules VALUES (?)");
  for (const id of rows) insert.run(id);
  const copy = join(directory, ".copy");
  mkdirSync(copy, { recursive: true });
  for (const suffix of ["", "-wal", "-shm"]) writeFileSync(join(copy, `${name}${suffix}`), readFileSync(`${path}${suffix}`));
  writer.close();
  for (const suffix of ["", "-wal", "-shm"]) {
    rmSync(`${path}${suffix}`, { force: true });
    if (existsSync(join(copy, `${name}${suffix}`))) writeFileSync(`${path}${suffix}`, readFileSync(join(copy, `${name}${suffix}`)));
  }
  rmSync(copy, { recursive: true, force: true });
  assert.ok(existsSync(`${path}-wal`) && statSync(`${path}-wal`).size > 0, "fixture keeps its rows in the WAL");
}

function ruleIds(path) {
  const database = new DatabaseSync(path, { readOnly: true });
  try {
    return database.prepare("SELECT id FROM catalog_rules ORDER BY id").all().map(row => row.id);
  } finally {
    database.close();
  }
}

test("--restore parsing takes an optional directory and leaves every other argument alone", () => {
  assert.deepEqual(parseRestoreArguments(["--project", "p"]), { restore: undefined, args: ["--project", "p"] });
  assert.deepEqual(parseRestoreArguments(["--restore"]), { restore: {}, args: [] });
  assert.deepEqual(parseRestoreArguments(["--restore", "--port", "1"]), { restore: {}, args: ["--port", "1"] });
  assert.deepEqual(parseRestoreArguments(["--restore", "/b", "--port", "1"]), { restore: { backupDir: "/b" }, args: ["--port", "1"] });
  assert.deepEqual(parseRestoreArguments(["--restore=/b"]), { restore: { backupDir: "/b" }, args: [] });
  assert.throws(() => parseRestoreArguments(["--restore", "--restore"]), /once/u);
  assert.throws(() => parseRestoreArguments(["--restore="]), /needs a backup directory/u);
});

test("backup selection takes the named directory or the newest one holding rerail-home/", async () => {
  await withHome(async home => {
    assert.throws(() => selectBackup({ userHome: home }), /No Rerail backup found/u);
    for (const name of [".rerail-clean-20260901", ".rerail-clean-20260929", ".rerail-clean-20260929-101500"]) {
      mkdirSync(join(home, name, "rerail-home"), { recursive: true });
    }
    mkdirSync(join(home, ".rerail-clean-20261231"), { recursive: true });
    assert.equal(selectBackup({ userHome: home }), join(home, ".rerail-clean-20260929-101500"));
    assert.equal(selectBackup({ userHome: home, backupDir: join(home, ".rerail-clean-20260901") }), join(home, ".rerail-clean-20260901"));
    assert.throws(() => selectBackup({ userHome: home, backupDir: join(home, ".rerail-clean-20261231") }), /not a just nuke-rerail backup/u);
  });
});

test("restore folds the backup's WAL in and moves the destination's database and stale sidecars aside", async () => {
  await withHome(async home => {
    const backup = join(home, ".rerail-clean-20260929");
    walOnlyDatabase(join(backup, "rerail-home"), "rerail.sqlite", ["kept-1", "kept-2", "kept-3"]);
    writeFileSync(join(backup, "rerail-home", "repository-id"), "home-id\n");
    mkdirSync(join(backup, "rerail-home", "sessions", "abc"), { recursive: true });
    writeFileSync(join(backup, "rerail-home", "sessions", "abc", "events.jsonl"), "{}\n");
    mkdirSync(join(backup, "rerail-home", "sessions", "claude-code-locks"), { recursive: true });
    const backupBefore = snapshot(backup);

    const rerailHome = join(home, ".rerail");
    walOnlyDatabase(rerailHome, "rerail.sqlite", ["fresh"]);
    writeFileSync(join(rerailHome, "rerail.sqlite-shm"), "stale shm");
    writeFileSync(join(rerailHome, "repository-id"), "fresh-id\n");

    const stamp = "20260929-120000";
    const { stagingDir, staged } = await stageDatabase(backup, join(home, "staging"));
    try {
      assert.ok(!existsSync(join(stagingDir, "rerail.sqlite-wal")));
      installIntoHome({ backupDir: backup, staged, home: rerailHome, stamp, includeHomeWorkspace: true });
    } finally {
      rmSync(stagingDir, { recursive: true, force: true });
    }

    assert.ok(!existsSync(join(rerailHome, "rerail.sqlite-wal")), "no stale WAL beside the restored file");
    assert.ok(!existsSync(join(rerailHome, "rerail.sqlite-shm")), "no stale shm beside the restored file");
    assert.deepEqual(ruleIds(join(rerailHome, "rerail.sqlite")), ["kept-1", "kept-2", "kept-3"]);
    for (const suffix of ["", "-wal", "-shm"]) assert.ok(existsSync(join(rerailHome, `rerail.sqlite${suffix}.pre-restore-${stamp}`)), suffix);
    assert.equal(readFileSync(join(rerailHome, "repository-id"), "utf8"), "home-id\n");
    assert.equal(readFileSync(join(rerailHome, `repository-id.pre-restore-${stamp}`), "utf8"), "fresh-id\n");
    assert.ok(existsSync(join(rerailHome, "sessions", "abc", "events.jsonl")));
    assert.ok(!existsSync(join(rerailHome, "sessions", "claude-code-locks")), "live-session locks are not restored");
    assert.deepEqual(snapshot(backup), backupBefore, "the backup is never modified");
  });
});

test("restoreBackup into a dev home restores the database only; a dry run changes nothing", async () => {
  await withHome(async home => {
    const backup = join(home, ".rerail-clean-20260929");
    walOnlyDatabase(join(backup, "rerail-home"), "rerail.sqlite", ["a", "b"]);
    writeFileSync(join(backup, "rerail-home", "repository-id"), "home-id\n");
    const devHome = join(home, "checkout", ".rerail", "dev", "home");
    mkdirSync(devHome, { recursive: true });

    const dry = await restoreBackup({ home: devHome, userHome: home, dryRun: true, env: envFor(home) });
    assert.equal(dry.backupDir, backup);
    assert.deepEqual(readdirSync(devHome), []);

    const result = await restoreBackup({ home: devHome, userHome: home, env: envFor(home) });
    assert.equal(result.backupDir, backup);
    assert.deepEqual(result.restored.map(entry => entry.target), [join(devHome, "rerail.sqlite")]);
    assert.deepEqual(result.rules, { table: "catalog_rules", count: 2 });
    assert.deepEqual(ruleIds(join(devHome, "rerail.sqlite")), ["a", "b"]);
    assert.ok(!existsSync(join(devHome, "repository-id")), "the home workspace's identity belongs to ~/.rerail");
  });
});

test("a backup without rerail.sqlite is refused", async () => {
  await withHome(async home => {
    mkdirSync(join(home, ".rerail-clean-20260929", "rerail-home"), { recursive: true });
    await assert.rejects(restoreBackup({ home: join(home, ".rerail"), userHome: home, env: envFor(home) }), /holds no rerail\.sqlite/u);
  });
});

test("restart reuses the port the daemon served before the restore", () => {
  assert.deepEqual(restartArguments(undefined), ["daemon", "start"]);
  assert.deepEqual(restartArguments({ port: 4800 }), ["daemon", "start", "--port", "4800"]);
});

test("stopDaemon without a launcher stops the recorded pid, and nothing recorded is not an error", async () => {
  await withHome(async home => {
    const rerailHome = join(home, ".rerail");
    mkdirSync(rerailHome, { recursive: true });
    assert.equal(await stopDaemon(rerailHome), "not-running");
    const child = spawn(process.execPath, ["-e", "setInterval(() => {}, 1000)"], { stdio: "ignore" });
    const exited = new Promise(resolve => child.once("exit", resolve));
    try {
      writeFileSync(join(rerailHome, "daemon.json"), JSON.stringify({ pid: child.pid }));
      assert.equal(await stopDaemon(rerailHome), "pid");
      await exited;
      assert.equal(processAlive(child.pid), false);
    } finally {
      child.kill("SIGKILL");
    }
  });
});

test("restoreInstalled stops the launcher's daemon, restores, restarts it on its port, and checks /healthz", async () => {
  await withHome(async home => {
    const server = createServer((request, response) => {
      response.setHeader("content-type", "application/json");
      response.end(JSON.stringify(request.url === "/healthz" ? { status: "ok" } : {}));
    });
    await new Promise(resolve => server.listen(0, "127.0.0.1", resolve));
    const port = server.address().port;
    try {
      const backup = join(home, ".rerail-clean-20260929");
      walOnlyDatabase(join(backup, "rerail-home"), "rerail.sqlite", ["r1", "r2"]);
      const rerailHome = join(home, ".rerail");
      const calls = join(home, "launcher-calls");
      const state = { pid: 999_999_999, port, url: `http://127.0.0.1:${port}` };
      mkdirSync(join(rerailHome, "bin"), { recursive: true });
      writeFileSync(join(rerailHome, "daemon.json"), JSON.stringify(state));
      writeFileSync(join(rerailHome, "bin", "rerail"), [
        "#!/bin/sh",
        `printf '%s %s\\n' "$RERAIL_HOME" "$*" >> '${calls}'`,
        `if [ "$1 $2" = "daemon stop" ]; then rm -f '${join(rerailHome, "daemon.json")}'; echo 'Rerail stopped.'; fi`,
        `if [ "$1 $2" = "daemon start" ]; then printf '%s' '${JSON.stringify(state)}' > '${join(rerailHome, "daemon.json")}'; fi`,
        "",
      ].join("\n"));
      chmodSync(join(rerailHome, "bin", "rerail"), 0o755);

      const lines = [];
      const result = await restoreInstalled({ backupDir: backup, env: envFor(home), log: line => lines.push(line) });
      assert.deepEqual(readFileSync(calls, "utf8").trim().split("\n"), [`${rerailHome} daemon stop`, `${rerailHome} daemon start --port ${port}`]);
      assert.deepEqual(result.rules, { table: "catalog_rules", count: 2 });
      assert.deepEqual(ruleIds(join(rerailHome, "rerail.sqlite")), ["r1", "r2"]);
      assert.ok(lines.some(line => line.includes(`Daemon healthy at http://127.0.0.1:${port}`)));
    } finally {
      server.close();
    }
  });
});

test("restoreInstalled refuses when setup left no launcher", async () => {
  await withHome(async home => {
    walOnlyDatabase(join(home, ".rerail-clean-20260929", "rerail-home"), "rerail.sqlite", ["r"]);
    await assert.rejects(
      restoreInstalled({ backupDir: join(home, ".rerail-clean-20260929"), env: envFor(home), log: () => {} }),
      /rerail is missing; rerail setup did not complete/u,
    );
  });
});
