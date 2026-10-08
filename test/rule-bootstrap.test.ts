import { spawn } from "node:child_process";
import { mkdir, mkdtemp, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase } from "../src/database.js";
import { codeRuleSchema, type CodeRule } from "../src/code-rule-schema.js";
import { loadBundledRules } from "../src/rule-bootstrap.js";
import { SqliteRuleStore, type BundledRuleDocument, type RuleDocument } from "../src/rule-store.js";
import { ruleDocumentDigest } from "../src/stable-json.js";
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

const bundled = (...rules: CodeRule[]): RuleDocument[] => rules.map(rule => ({ kind: "file", rule }));

describe("SqliteRuleStore.seed", () => {
  it("inserts new bundled rules with their digest and leaves them alone on the next run", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    const documents = bundled(codeRule("bundled:a"), codeRule("bundled:b"));

    expect(store.seed(documents, 1_000)).toEqual({ inserted: 2, refreshed: 0, kept: 0 });
    expect(store.get("bundled:a")).toMatchObject({ sourceDigest: ruleDocumentDigest(documents[0]!), createdAt: 1_000 });
    expect(store.seed(documents, 2_000)).toEqual({ inserted: 0, refreshed: 0, kept: 2 });
    expect(store.get("bundled:a")?.updatedAt).toBe(1_000);
    store.close();
  });

  it("follows an upstream change only while the row is untouched, and never overwrites an edit", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    store.seed(bundled(codeRule("bundled:a"), codeRule("bundled:b")), 1_000);
    store.setEnabled({ ids: ["bundled:b"] }, false, 1_500);
    // The user edits rule a in place; rule b only had its enabled flag flipped, which is state, not content.
    store.upsertCodeRule(codeRule("bundled:a", 1, "Use the logger"), { now: 2_000 });

    const upstream = bundled(codeRule("bundled:a", 2, "Print is forbidden"), codeRule("bundled:b", 2, "Print is forbidden"));
    expect(store.seed(upstream, 3_000)).toEqual({ inserted: 0, refreshed: 1, kept: 1 });
    const edited = store.get("bundled:a");
    expect(edited?.kind === "file" && edited.rule.clauses[0]?.violation?.message).toBe("Use the logger");
    expect(edited?.revision).toBe(1);
    const refreshed = store.get("bundled:b");
    expect(refreshed?.revision).toBe(2);
    expect(refreshed?.enabled).toBe(false);
    expect(refreshed?.sourceDigest).toBe(ruleDocumentDigest(upstream[1]!));
    store.close();
  });

  it("inserts an opt-in document disabled, and neither a refresh nor a re-seed changes its enabled state", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    const optIn = (rule: CodeRule): BundledRuleDocument => ({ kind: "file", rule, enabledByDefault: false });

    expect(store.seed([optIn(codeRule("pack:a")), optIn(codeRule("pack:b"))], 1_000)).toEqual({ inserted: 2, refreshed: 0, kept: 0 });
    expect(store.get("pack:a")).toMatchObject({ enabled: false, sourceDigest: ruleDocumentDigest({ kind: "file", rule: codeRule("pack:a") }) });
    expect(store.codeRules()).toEqual([]);
    store.setEnabled({ ids: ["pack:b"] }, true, 1_500);

    expect(store.seed([optIn(codeRule("pack:a", 2)), optIn(codeRule("pack:b", 2))], 2_000)).toEqual({ inserted: 0, refreshed: 2, kept: 0 });
    expect(store.get("pack:a")).toMatchObject({ enabled: false, revision: 2 });
    expect(store.get("pack:b")).toMatchObject({ enabled: true, revision: 2 });
    expect(store.codeRules().map(rule => rule.id)).toEqual(["pack:b"]);
    store.close();
  });

  it("never touches a user-created rule that collides with a new bundled id", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    store.upsertCodeRule(codeRule("shared:id", 7, "Mine"), { now: 1_000 });
    expect(store.seed(bundled(codeRule("shared:id")), 2_000)).toEqual({ inserted: 0, refreshed: 0, kept: 1 });
    expect(store.get("shared:id")).toMatchObject({ revision: 7, sourceDigest: null });
    store.close();
  });
});

/** A sanity floor, not the exact count. */
const MINIMUM_BUNDLED_RULES = 10;

describe("bundled rules", () => {
  it("ships only opt-in packs: every document parses as a file rule with a unique <pack>:<rule> id, disabled by default", () => {
    const documents = loadBundledRules();
    expect(documents.length).toBeGreaterThan(MINIMUM_BUNDLED_RULES);
    expect(new Set(documents.map(document => document.rule.id)).size).toBe(documents.length);
    expect(documents.every(document => document.kind === "file")).toBe(true);
    expect(documents.every(document => document.enabledByDefault === false)).toBe(true);
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
    expect(loadBundledRules(root).map(document => [document.rule.id, document.enabledByDefault])).toEqual([
      ["example:default", undefined],
      ["alpha:a", false],
      ["alpha:b", false],
      ["zeta:a", false],
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
    expect(loadBundledRules(root).map(document => [document.rule.id, document.enabledByDefault])).toEqual([
      ["example:default", undefined],
      ["tslint:a", false],
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
      expect(created.prepare("SELECT count(*) AS tables FROM sqlite_master WHERE type = 'table' AND name = 'catalog_rules'").get()).toEqual({ tables: 1 });
      created.close();
    }
  }, 60_000);
});
