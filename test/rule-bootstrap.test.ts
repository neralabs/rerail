import { spawn } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase } from "../src/database.js";
import { codeRuleSchema, type CodeRule } from "../src/code-rule-schema.js";
import { loadBundledRules } from "../src/bundled-rule-documents.js";
import { RERAIL_SCHEMA_VERSION } from "../src/schema.js";

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

/** A sanity floor, not the exact count. */
const MINIMUM_BUNDLED_RULES = 10;

describe("bundled rules", () => {
  it("ships only opt-in packs: every document parses as a file rule with a unique <pack>:<rule> id", () => {
    const documents = loadBundledRules();
    expect(documents.length).toBeGreaterThan(MINIMUM_BUNDLED_RULES);
    expect(new Set(documents.map(document => document.rule.id)).size).toBe(documents.length);
    expect(documents.every(document => document.kind === "file")).toBe(true);
    expect(documents.every(document => /^(?:complexity|tslint):[a-z0-9-]+$/u.test(document.rule.id))).toBe(true);
    expect(documents.map(document => document.rule.id)).not.toContain("tslint:file-header");
  });

  it("reads opt-in packs from opt-in/<pack>/ by pack then file name, ignores non-JSON files in a pack, and rejects a file outside a pack or a duplicate id", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-bundled-"));
    temporaryDirectories.push(root);
    const write = async (path: string, id: string): Promise<void> => {
      await mkdir(dirname(join(root, path)), { recursive: true });
      await writeFile(join(root, path), JSON.stringify(codeRule(id)));
    };
    await write("default.json", "example:default");
    await write("opt-in/zeta/a.json", "zeta:a");
    await write("opt-in/alpha/b.json", "alpha:b");
    await write("opt-in/alpha/a.json", "alpha:a");
    await writeFile(join(root, "opt-in", "alpha", "README.md"), "# alpha pack\n");
    expect(loadBundledRules(root).map(document => document.rule.id)).toEqual([
      "example:default",
      "alpha:a",
      "alpha:b",
      "zeta:a",
    ]);

    await write("opt-in/zeta/b.json", "alpha:a");
    expect(() => loadBundledRules(root)).toThrow(/Duplicate bundled rule id: alpha:a/u);
    await rm(join(root, "opt-in", "zeta", "b.json"));

    await write("opt-in/tslint-stray.json", "tslint:stray");
    expect(() => loadBundledRules(root)).toThrow(/must live in a pack folder .*tslint-stray\.json/u);
  });

  it("ignores hidden entries and non-JSON files directly in opt-in/, such as .DS_Store or a README", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-bundled-"));
    temporaryDirectories.push(root);
    await mkdir(join(root, "opt-in", "tslint"), { recursive: true });
    await mkdir(join(root, "opt-in", ".hidden-pack"), { recursive: true });
    await writeFile(join(root, "default.json"), JSON.stringify(codeRule("example:default")));
    await writeFile(join(root, "opt-in", "tslint", "a.json"), JSON.stringify(codeRule("tslint:a")));
    await writeFile(join(root, "opt-in", "tslint", "README.md"), "# tslint pack\n");
    await writeFile(join(root, "opt-in", ".hidden-pack", "x.json"), JSON.stringify(codeRule("hidden:x")));
    await writeFile(join(root, "opt-in", ".DS_Store"), "\u0000\u0001binary");
    await writeFile(join(root, "opt-in", "README.md"), "# opt-in packs\n");
    expect(loadBundledRules(root).map(document => document.rule.id)).toEqual([
      "example:default",
      "tslint:a",
    ]);
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
      import { openRerailDatabase } from ${JSON.stringify(pathToFileURL(resolve("dist/database.js")).href)};
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
