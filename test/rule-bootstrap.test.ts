import { spawn } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { mkdir, mkdtemp, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { pathToFileURL } from "node:url";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase } from "../src/database.js";
import { policyRuleSchema, type PolicyRule } from "../src/policy-schema.js";
import { bootstrapRules, defaultRulesDumpPath, exportRules, importRules, loadBundledRules, readRulesDump } from "../src/rule-bootstrap.js";
import { SqliteRuleStore, type BundledRuleDocument, type RuleDocument } from "../src/rule-store.js";
import { ruleDocumentDigest } from "../src/stable-json.js";
import { RERAIL_SCHEMA_COMPATIBILITY_FLOOR, RERAIL_SCHEMA_VERSION } from "../src/schema.js";
import { MOCK_COSINE_RULES } from "../src/seed.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryDatabase(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-bootstrap-"));
  temporaryDirectories.push(root);
  return join(root, ".rerail", "rerail.sqlite");
}

function policy(id: string, revision = 1, message = "No print"): PolicyRule {
  return policyRuleSchema.parse({
    checks: [
      {
        id: `${id}:check`,
        finding: { anchor: "witness.span", code: "FORBIDDEN_PRINT", message, severity: "error" },
        forbid: { regex: { pattern: "\\bprint\\(", source: "file.content" } },
      },
    ],
    format: "rerail.policy/v1",
    id,
    revision,
  });
}

const bundled = (...rules: PolicyRule[]): RuleDocument[] => rules.map(rule => ({ kind: "policy", rule }));

describe("SqliteRuleStore.seed", () => {
  it("inserts new bundled rules with their digest and leaves them alone on the next run", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    const documents = bundled(policy("bundled:a"), policy("bundled:b"));

    expect(store.seed(documents, 1_000)).toEqual({ inserted: 2, refreshed: 0, kept: 0 });
    expect(store.get("bundled:a")).toMatchObject({ sourceDigest: ruleDocumentDigest(documents[0]!), createdAt: 1_000 });
    expect(store.seed(documents, 2_000)).toEqual({ inserted: 0, refreshed: 0, kept: 2 });
    expect(store.get("bundled:a")?.updatedAt).toBe(1_000);
    store.close();
  });

  it("follows an upstream change only while the row is untouched, and never overwrites an edit", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    store.seed(bundled(policy("bundled:a"), policy("bundled:b")), 1_000);
    store.setEnabled({ ids: ["bundled:b"] }, false, 1_500);
    // The user edits rule a in place; rule b only had its enabled flag flipped, which is state, not content.
    store.upsertPolicy(policy("bundled:a", 1, "Use the logger"), { now: 2_000 });

    const upstream = bundled(policy("bundled:a", 2, "Print is forbidden"), policy("bundled:b", 2, "Print is forbidden"));
    expect(store.seed(upstream, 3_000)).toEqual({ inserted: 0, refreshed: 1, kept: 1 });
    const edited = store.get("bundled:a");
    expect(edited?.kind === "policy" && edited.rule.checks[0]?.finding?.message).toBe("Use the logger");
    expect(edited?.revision).toBe(1);
    const refreshed = store.get("bundled:b");
    expect(refreshed?.revision).toBe(2);
    expect(refreshed?.enabled).toBe(false);
    expect(refreshed?.sourceDigest).toBe(ruleDocumentDigest(upstream[1]!));
    store.close();
  });

  it("inserts an opt-in document disabled, and neither a refresh nor a re-seed changes its enabled state", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    const optIn = (rule: PolicyRule): BundledRuleDocument => ({ kind: "policy", rule, enabledByDefault: false });

    expect(store.seed([optIn(policy("pack:a")), optIn(policy("pack:b"))], 1_000)).toEqual({ inserted: 2, refreshed: 0, kept: 0 });
    expect(store.get("pack:a")).toMatchObject({ enabled: false, sourceDigest: ruleDocumentDigest({ kind: "policy", rule: policy("pack:a") }) });
    expect(store.policyRules()).toEqual([]);
    store.setEnabled({ ids: ["pack:b"] }, true, 1_500);

    expect(store.seed([optIn(policy("pack:a", 2)), optIn(policy("pack:b", 2))], 2_000)).toEqual({ inserted: 0, refreshed: 2, kept: 0 });
    expect(store.get("pack:a")).toMatchObject({ enabled: false, revision: 2 });
    expect(store.get("pack:b")).toMatchObject({ enabled: true, revision: 2 });
    expect(store.policyRules().map(rule => rule.id)).toEqual(["pack:b"]);
    store.close();
  });

  it("never touches a user-created rule that collides with a new bundled id", async () => {
    const store = new SqliteRuleStore(await temporaryDatabase());
    store.upsertPolicy(policy("shared:id", 7, "Mine"), { now: 1_000 });
    expect(store.seed(bundled(policy("shared:id")), 2_000)).toEqual({ inserted: 0, refreshed: 0, kept: 1 });
    expect(store.get("shared:id")).toMatchObject({ revision: 7, sourceDigest: null });
    store.close();
  });
});

/** A sanity floor, not the exact count: bundled-rules.test.ts checks every document against its fixtures. */
const MINIMUM_BUNDLED_RULES = 10;

describe("rules dump", () => {
  it("round-trips every row, including provenance, state, and timestamps", async () => {
    const path = await temporaryDatabase();
    const store = new SqliteRuleStore(path);
    store.seed(bundled(policy("bundled:a")), 1_000);
    store.upsertCosine(MOCK_COSINE_RULES[0], { now: 1_500, revision: 3, enabled: false, scope: "global" });
    const before = store.list();
    const dumpPath = defaultRulesDumpPath(path);
    expect(exportRules(store, dumpPath, new Date("2026-09-08T00:00:00.000Z"))).toBe(2);
    store.close();

    const dump = readRulesDump(dumpPath);
    expect(dump).toMatchObject({ version: 1, exportedAt: "2026-09-08T00:00:00.000Z" });
    expect(JSON.parse(readFileSync(dumpPath, "utf8")).rules).toHaveLength(2);

    const restoredPath = join(temporaryDirectories[0]!, "restored.sqlite");
    const restored = new SqliteRuleStore(restoredPath);
    expect(importRules(restored, dumpPath)).toBe(2);
    expect(restored.list()).toEqual(before);
    restored.close();
  });

  it("restores the dump into an empty database before seeding, and skips it otherwise", async () => {
    const path = await temporaryDatabase();
    const first = new SqliteRuleStore(path);
    first.seed(bundled(policy("bundled:a")), 1_000);
    first.upsertPolicy(policy("bundled:a", 1, "Use the logger"), { now: 2_000 });
    const dumpPath = defaultRulesDumpPath(path);
    exportRules(first, dumpPath);
    first.close();

    // A reset: the database is gone, the dump is not.
    await rm(path);
    const fresh = new SqliteRuleStore(path);
    const report = bootstrapRules(fresh, { bundled: bundled(policy("bundled:a", 2), policy("bundled:b")), dumpPath });
    expect(report).toEqual({ restored: 1, inserted: 1, refreshed: 0, kept: 1, dumpPath });
    const edited = fresh.get("bundled:a");
    expect(edited?.kind === "policy" && edited.rule.checks[0]?.finding?.message).toBe("Use the logger");
    expect(fresh.get("bundled:b")).toBeDefined();

    // A populated table ignores the dump even when it still exists.
    fresh.remove(["bundled:b"]);
    expect(bootstrapRules(fresh, { bundled: [], dumpPath })).toMatchObject({ restored: 0 });
    expect(fresh.get("bundled:b")).toBeUndefined();
    fresh.close();
  });

  it("defaults to the bundled documents, which all parse, carry unique ids, and are grouped by a nera-<group> prefix", () => {
    const documents = loadBundledRules();
    const defaults = documents.filter(document => document.enabledByDefault !== false);
    expect(defaults.length).toBeGreaterThan(MINIMUM_BUNDLED_RULES);
    expect(new Set(documents.map(document => document.rule.id)).size).toBe(documents.length);
    expect(documents.every(document => document.kind === "policy")).toBe(true);
    expect(defaults.every(document => /^nera-[a-z-]+:[a-z0-9-]+$/u.test(document.rule.id))).toBe(true);
  });

  it("loads the opt-in pack after the defaults, marked disabled by default and namespaced tslint:<rule>", () => {
    const documents = loadBundledRules();
    const optIn = documents.filter(document => document.enabledByDefault === false);
    expect(optIn.length).toBeGreaterThan(0);
    expect(optIn.every(document => /^tslint:[a-z0-9-]+$/u.test(document.rule.id))).toBe(true);
    expect(optIn.map(document => document.rule.id)).not.toContain("tslint:file-header");
    expect(documents.slice(documents.length - optIn.length)).toEqual(optIn);
  });

  it("reads opt-in packs from opt-in/<pack>/ by pack then file name, ignores non-JSON files in a pack, and rejects a file outside a pack or a duplicate id", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-bundled-"));
    temporaryDirectories.push(root);
    const write = async (path: string, id: string): Promise<void> => {
      await mkdir(dirname(join(root, path)), { recursive: true });
      await writeFile(join(root, path), JSON.stringify(policy(id)));
    };
    await write("default.json", "nera-core:default");
    await write("opt-in/zeta/a.json", "zeta:a");
    await write("opt-in/alpha/b.json", "alpha:b");
    await write("opt-in/alpha/a.json", "alpha:a");
    await writeFile(join(root, "opt-in", "alpha", "README.md"), "# alpha pack\n");
    expect(loadBundledRules(root).map(document => [document.rule.id, document.enabledByDefault])).toEqual([
      ["nera-core:default", undefined],
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
    await writeFile(join(root, "default.json"), JSON.stringify(policy("nera-core:default")));
    await writeFile(join(root, "opt-in", "tslint", "a.json"), JSON.stringify(policy("tslint:a")));
    await writeFile(join(root, "opt-in", "tslint", "README.md"), "# tslint pack\n");
    await writeFile(join(root, "opt-in", ".hidden-pack", "x.json"), JSON.stringify(policy("hidden:x")));
    await writeFile(join(root, "opt-in", ".DS_Store"), "\u0000\u0001binary");
    await writeFile(join(root, "opt-in", "README.md"), "# opt-in packs\n");
    expect(loadBundledRules(root).map(document => [document.rule.id, document.enabledByDefault])).toEqual([
      ["nera-core:default", undefined],
      ["tslint:a", false],
    ]);
  });
});

describe("schema upgrades", () => {
  it("stamps a fresh database with its compatibility floor and additive revision", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    const stamped = new Database(path, { readonly: true });
    expect(stamped.pragma("user_version", { simple: true })).toBe(RERAIL_SCHEMA_COMPATIBILITY_FLOOR);
    expect(stamped.prepare("SELECT revision FROM rerail_schema_revision WHERE singleton = 1").get()).toEqual({ revision: RERAIL_SCHEMA_VERSION });
    expect(stamped.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'policy_run_result_pages'").get()).toEqual({
      name: "policy_run_result_pages",
    });
    stamped.close();
  });

  it("restamps a pre-floor v11 database with its v10 compatibility floor", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    const legacy = new Database(path);
    legacy.exec("DROP TABLE rerail_schema_revision; PRAGMA user_version = 11");
    legacy.close();

    openRerailDatabase(path).$client.close();
    const upgraded = new Database(path, { readonly: true });
    expect(upgraded.pragma("user_version", { simple: true })).toBe(RERAIL_SCHEMA_COMPATIBILITY_FLOOR);
    expect(upgraded.prepare("SELECT revision FROM rerail_schema_revision WHERE singleton = 1").get()).toEqual({ revision: RERAIL_SCHEMA_VERSION });
    upgraded.close();
  });

  it("restamps an unreleased pre-floor v12 single-database file and keeps its repository attribution", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    const branch = new Database(path);
    branch.exec("DROP TABLE policy_run_result_pages; DROP TABLE rerail_schema_revision; PRAGMA user_version = 12");
    branch.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at) VALUES ('s', 'omp', '/r', 'repo', 1)").run();
    branch.close();

    openRerailDatabase(path).$client.close();
    const upgraded = new Database(path, { readonly: true });
    expect(upgraded.pragma("user_version", { simple: true })).toBe(RERAIL_SCHEMA_COMPATIBILITY_FLOOR);
    expect(upgraded.prepare("SELECT revision FROM rerail_schema_revision WHERE singleton = 1").get()).toEqual({ revision: RERAIL_SCHEMA_VERSION });
    expect(upgraded.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name = 'policy_run_result_pages'").get()).toBeDefined();
    expect(upgraded.prepare("SELECT repository_id AS repositoryId FROM sessions").all()).toEqual([{ repositoryId: "repo" }]);
    upgraded.close();
  });

  it("refuses a database whose compatibility floor needs a newer Rerail", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    const newer = new Database(path);
    newer.exec(`PRAGMA user_version = ${RERAIL_SCHEMA_VERSION + 1}`);
    newer.close();

    expect(() => openRerailDatabase(path)).toThrow("requires a newer Rerail");
  });

  it("opens a newer additive revision without downgrading it", async () => {
    const path = await temporaryDatabase();
    openRerailDatabase(path).$client.close();
    const newer = new Database(path);
    newer.exec(`UPDATE rerail_schema_revision SET revision = ${RERAIL_SCHEMA_VERSION + 1}; PRAGMA user_version = 10`);
    newer.close();

    openRerailDatabase(path).$client.close();
    const opened = new Database(path, { readonly: true });
    expect(opened.pragma("user_version", { simple: true })).toBe(RERAIL_SCHEMA_COMPATIBILITY_FLOOR);
    expect(opened.prepare("SELECT revision FROM rerail_schema_revision WHERE singleton = 1").get()).toEqual({ revision: RERAIL_SCHEMA_VERSION + 1 });
    opened.close();
  });

  it("adds source_digest to a v1 database without losing its rows, and drops the v2 cache tables", async () => {
    const path = await temporaryDatabase();
    await mkdir(dirname(path), { recursive: true });
    // Rebuild the v1 shape by hand: no stamp, no source_digest, the cache table v2 replaced and v3 removed.
    const legacy = new Database(path);
    legacy.exec(`
      CREATE TABLE rules (id text PRIMARY KEY, kind text NOT NULL, revision integer NOT NULL, enabled integer NOT NULL DEFAULT true,
        scope text NOT NULL DEFAULT 'repository', created_at integer NOT NULL, updated_at integer NOT NULL, format text, description text,
        scope_json text, checks_json text, reference text, match_json text, sample_json text, action_json text, repeat_json text);
      CREATE TABLE policy_dsl_catalog (singleton integer PRIMARY KEY, catalog_digest text NOT NULL, capabilities_json text NOT NULL,
        rule_count integer NOT NULL, CONSTRAINT policy_dsl_catalog_rule_count CHECK (rule_count > 0));
      INSERT INTO rules (id, kind, revision, enabled, scope, created_at, updated_at, format, checks_json)
        VALUES ('old:rule', 'policy', 1, 1, 'repository', 1, 1, 'rerail.policy/v1', '[]');
    `);
    legacy.close();

    openRerailDatabase(path).$client.close();
    const upgraded = new Database(path, { readonly: true });
    expect(upgraded.pragma("user_version", { simple: true })).toBe(RERAIL_SCHEMA_COMPATIBILITY_FLOOR);
    expect(upgraded.prepare("SELECT id, source_digest AS sourceDigest FROM rules").all()).toEqual([{ id: "old:rule", sourceDigest: null }]);
    expect(upgraded.prepare("SELECT name FROM sqlite_master WHERE name LIKE 'policy_dsl_%'").all()).toEqual([]);
    upgraded.close();
    expect(existsSync(`${path}-wal`)).toBe(true);
  });

  // contract: any number of processes may open the same v1 file at once; every one succeeds, the column is
  // added exactly once, and the file ends in WAL. The WAL switch races (SQLite skips the busy handler on the
  // deadlock-avoidance path), so this runs enough openers that a missing wait cannot slip through by luck:
  // roughly one opener in seven fails without it, so 48 openers leave well under a 0.1% chance of a false pass.
  it("upgrades a v1 database once when many processes open it at the same time", async () => {
    const opener = `
      import { openRerailDatabase } from ${JSON.stringify(pathToFileURL(resolve("dist/database.js")).href)};
      openRerailDatabase(process.argv[1]).$client.close();
    `;
    for (let round = 0; round < 6; round += 1) {
      const path = await temporaryDatabase();
      await mkdir(dirname(path), { recursive: true });
      const legacy = new Database(path);
      legacy.exec(`
        CREATE TABLE rules (id text PRIMARY KEY, kind text NOT NULL, revision integer NOT NULL, enabled integer NOT NULL DEFAULT true,
          scope text NOT NULL DEFAULT 'repository', created_at integer NOT NULL, updated_at integer NOT NULL, format text, description text,
          scope_json text, checks_json text, reference text, match_json text, sample_json text, action_json text, repeat_json text);
      `);
      legacy.close();
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
      const upgraded = new Database(path, { readonly: true });
      expect(upgraded.pragma("journal_mode", { simple: true })).toBe("wal");
      expect(upgraded.pragma("user_version", { simple: true })).toBe(RERAIL_SCHEMA_COMPATIBILITY_FLOOR);
      expect(upgraded.prepare("PRAGMA table_info(rules)").all().filter(column => (column as { name: string }).name === "source_digest")).toHaveLength(1);
      upgraded.close();
    }
  }, 60_000);
});
