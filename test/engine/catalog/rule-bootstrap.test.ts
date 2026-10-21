import { spawn } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase } from "../../../src/engine/storage/database.js";
import { codeRuleSchema, type CodeRule } from "../../../src/engine/code-rule/code-rule-schema.js";
import { loadPackRuleDocuments } from "../../../src/engine/catalog/rule-documents.js";
import { RERAIL_SCHEMA_VERSION } from "../../../src/engine/storage/migrations.js";
import { exportRulesDump, importRulesDump, type RulesDump } from "../../../src/engine/catalog/rule-bootstrap.js";
import { testBundle } from "../../support/rule-configuration-builders.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryDatabase(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-bootstrap-"));
  temporaryDirectories.push(root);
  return join(root, ".rerail", "rerail.sqlite");
}

function codeRule(id: string, revision = 1, message = "No print"): CodeRule {
  return codeRuleSchema.parse({
    clauses: [
      {
        id: `${id}:check`,
        violation: { anchor: "witness.span", code: "FORBIDDEN_PRINT", message },
        forbid: { regex: { pattern: "\\bprint\\(", source: "file.content" } },
      },
    ],
    format: "rerail.policy/v1",
    id,
    revision,
    mode: "block",
  });
}

describe("pack rule documents", () => {
  it("reads each pack's rules/ in pack order and by file name, ignoring non-JSON files and a pack without rules/, and rejects a duplicate id", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-pack-documents-"));
    temporaryDirectories.push(root);
    const write = async (path: string, id: string): Promise<void> => {
      await mkdir(dirname(join(root, path)), { recursive: true });
      await writeFile(join(root, path), JSON.stringify(codeRule(id)));
    };
    await write("packs/zeta/rules/a.json", "zeta:a");
    await write("packs/alpha/rules/b.json", "alpha:b");
    await write("packs/alpha/rules/a.json", "alpha:a");
    await writeFile(join(root, "packs", "alpha", "rules", "README.md"), "# alpha pack\n");
    await writeFile(join(root, "packs", "alpha", "rules", ".DS_Store"), "\u0000\u0001binary");
    const packs = [join(root, "packs", "alpha"), join(root, "packs", "absent"), join(root, "packs", "zeta")];
    expect(loadPackRuleDocuments(packs).map(document => document.rule.id)).toEqual([
      "alpha:a",
      "alpha:b",
      "zeta:a",
    ]);

    await write("packs/zeta/rules/b.json", "alpha:a");
    expect(() => loadPackRuleDocuments(packs)).toThrow(/Duplicate rule id: alpha:a/u);
  });
});

describe("database schema", () => {
  it("creates and stamps a fresh database with the current schema version", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    const stamped = new Database(path, { readonly: true });
    expect(stamped.pragma("user_version", { simple: true })).toBe(RERAIL_SCHEMA_VERSION);
    expect(stamped.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'audit_result_pages'").get()).toEqual({
      name: "audit_result_pages",
    });
    stamped.close();
  });

  it("refuses a database written by a newer Rerail and leaves its stamp alone", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    const newer = new Database(path);
    newer.exec(`PRAGMA user_version = ${RERAIL_SCHEMA_VERSION + 1}`);
    newer.close();

    expect(() => openRerailDatabase(path)).toThrow("written by a newer Rerail");
    const reopened = new Database(path, { readonly: true });
    expect(reopened.pragma("user_version", { simple: true })).toBe(RERAIL_SCHEMA_VERSION + 1);
    reopened.close();
  });

  // contract: any number of processes may create the same database at once; every one succeeds and the file
  // ends stamped and in WAL. The WAL switch races (SQLite skips the busy handler on the deadlock-avoidance
  // path), so this runs enough openers that a missing wait cannot slip through by luck: roughly one opener in
  // seven fails without it, so 48 openers leave well under a 0.1% chance of a false pass.
  it("creates one database when many processes open it at the same time", async () => {
    const opener = `
      import { openRerailDatabase } from ${JSON.stringify(pathToFileURL(resolve("dist/engine/storage/database.js")).href)};
      openRerailDatabase(process.argv[1]).$client.close();
    `;
    for (let round = 0; round < 6; round += 1) {
      const path = await temporaryDatabase();
      await mkdir(dirname(path), { recursive: true });
      const results = await Promise.all(
        Array.from({ length: 8 }, () => {
          const done = Promise.withResolvers<{ code: number | null; stderr: string }>();
          const child = spawn(process.execPath, ["--input-type=module", "-e", opener, path], { stdio: ["ignore", "ignore", "pipe"] });
          let stderr = "";
          child.stderr.on("data", chunk => { stderr += String(chunk); });
          child.once("close", code => done.resolve({ code, stderr }));
          return done.promise;
        }),
      );
      expect(results.map(result => result.stderr).join("")).toBe("");
      expect(results.map(result => result.code)).toEqual(Array.from({ length: 8 }, () => 0));
      const created = new Database(path, { readonly: true });
      expect(created.pragma("journal_mode", { simple: true })).toBe("wal");
      expect(created.pragma("user_version", { simple: true })).toBe(RERAIL_SCHEMA_VERSION);
      expect(created.prepare("SELECT count(*) AS tables FROM sqlite_master WHERE type = 'table' AND name = 'library_rules'").get()).toEqual({ tables: 1 });
      created.close();
    }
  }, 60_000);
});

describe("rules dump", () => {
  // Self-review round 5: a later Rerail may store a setting that sets only fields this one does not know; exporting it
  // with on and mode both null made this version's own import refuse the dump.
  it("leaves out a setting that sets neither on nor mode, so the dump imports", async () => {
    const path = await temporaryDatabase();
    const source = openRerailDatabase(path);
    let dump: unknown;
    try {
      source.$client.exec("INSERT INTO repositories (repository_key, display_name, created_at, updated_at) VALUES ('repo', 'repo', 0, 0)");
      const insert = source.$client.prepare(
        "INSERT INTO rule_settings (owner_kind, owner_id, folder, target_kind, target_id, \"on\", mode, updated_at) VALUES ('repository', 'repo', ?, 'pack', 'lint', ?, NULL, 0)",
      );
      insert.run("", 0);
      // The later version's table allows it; this one's CHECK does not.
      source.$client.pragma("ignore_check_constraints = ON");
      insert.run("web", null);
      source.$client.pragma("ignore_check_constraints = OFF");
      dump = JSON.parse(JSON.stringify(exportRulesDump(source)));
    } finally {
      source.$client.close();
    }
    expect((dump as RulesDump).repositories[0]?.settings).toEqual([{ folder: "", targetKind: "pack", targetId: "lint", on: false, mode: null }]);

    const target = openRerailDatabase(await temporaryDatabase());
    try {
      expect(importRulesDump(target, testBundle(), dump)).toEqual({ library: 0, repositories: 1 });
      expect(exportRulesDump(target).repositories[0]?.settings).toEqual([{ folder: "", targetKind: "pack", targetId: "lint", on: false, mode: null }]);
    } finally {
      target.$client.close();
    }
  });
});
