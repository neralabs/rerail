import { existsSync } from "node:fs";
import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { deserialize, serialize } from "node:v8";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it, vi } from "vitest";
import { openRerailDatabase, resetRepositoryHistory } from "../src/database.js";
import { SqlitePolicyExecutionReader, SqlitePolicyExecutionStore } from "../src/policy-execution-store.js";
import { policyRuleSchema, type PolicyRule } from "../src/policy-schema.js";
import { loadBundledRules } from "../src/rule-bootstrap.js";
import type { CosineRule } from "../src/rule-schema.js";
import { SqliteRuleStore, type RuleDocument } from "../src/rule-store.js";
import { SqliteScanControlStore } from "../src/scan-control.js";
import { RERAIL_SCHEMA_COMPATIBILITY_FLOOR, RERAIL_SCHEMA_VERSION } from "../src/schema.js";
import { ScopedRuleCatalog, scopedRuleSignature } from "../src/scoped-rule-catalog.js";
import { MOCK_COSINE_RULES } from "../src/seed.js";
import { ruleDocumentDigest, stableJsonText } from "../src/stable-json.js";
import { createV14Database, v14TableStatement } from "./support/v14-database.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryDirectory(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-v15-"));
  temporaryDirectories.push(root);
  return root;
}

async function temporaryDatabase(name = "transcripts.sqlite"): Promise<string> {
  return join(await temporaryDirectory(), ".rerail", name);
}

/** Digest of a document under any kind a schema has stored. */
const digestOf = (kind: string, rule: unknown): string => ruleDocumentDigest({ kind, rule } as unknown as RuleDocument);

const BUNDLED = loadBundledRules().flatMap(document => (document.kind === "file" ? [document.rule] : []));
const BUNDLED_POLICY: PolicyRule = BUNDLED[0]!;
const EDITED_POLICY: PolicyRule = BUNDLED[1]!;
const COSINE: CosineRule = MOCK_COSINE_RULES[0]!;
/** A define clause reports nothing, so it has no violation; rule modes and the contract leave it as it is. */
const DEFINE_CLAUSE = { id: "options", define: { options: { strict: { type: "boolean", default: true } } } };
const DEFINE_POLICY: PolicyRule = policyRuleSchema.parse({ ...BUNDLED[2]!, id: "with-define", clauses: [DEFINE_CLAUSE, ...BUNDLED[2]!.clauses] });

/** A policy body exactly as v14 wrote it: the v14 format, `checks`, and each clause's `finding`. */
function v14Policy(rule: PolicyRule): Record<string, unknown> {
  const { clauses, ...body } = rule;
  return {
    ...body,
    format: "rerail.policy/v1",
    checks: clauses.map(({ violation, ...clause }) => (violation === undefined ? clause : { ...clause, finding: violation })),
  };
}

/** The severity main's v13 stored on every reporting check of a rule that now has `mode`. */
function legacySeverity(mode: string): string {
  if (mode === "log") return "advisory";
  if (mode === "steer") return "warning";
  throw new Error(`no severity before rule modes reads as ${mode}`);
}

/** A policy body as main's v13 stored it: no rule mode, the v14 format and keys, a severity on each reporting check. */
function mainPolicy(rule: PolicyRule): Record<string, unknown> {
  const { mode, clauses, ...body } = rule;
  const severity = legacySeverity(mode);
  return {
    ...body,
    format: "rerail.policy/v1",
    checks: clauses.map(({ violation, ...clause }) => (violation === undefined ? clause : { ...clause, finding: { ...violation, severity } })),
  };
}

/** A cosine body as main's v13 stored it: no rule mode, `action.delivery` instead. */
function mainCosine(rule: CosineRule): Record<string, unknown> {
  const { mode, action, ...body } = rule;
  return { ...body, action: { delivery: mode === "log" ? "observe" : "steer", ...action } };
}

/** The `rules` table before rule modes: the v14 DDL without the `mode` column and its CHECK. */
function rulesTableWithoutMode(): string {
  const current = v14TableStatement("rules");
  const column = "\n\t`mode` text NOT NULL,";
  const constraint = `\n\tCONSTRAINT "rules_mode" CHECK("rules"."mode" IN ('log', 'steer', 'block')),`;
  expect(current).toContain(column);
  expect(current).toContain(constraint);
  return current.replace(column, "").replace(constraint, "");
}

/** Put `repositoryId` in its own project so a catalog opened for it sees the project's rules. */
function insertRepositoryProject(database: Database.Database, repositoryId: string): void {
  database.prepare("INSERT INTO rule_projects (project_id, name, created_at, updated_at) VALUES (?, ?, 1, 1)").run(`repository:${repositoryId}`, repositoryId);
  database.prepare("INSERT INTO rule_repositories (repository_id, last_known_path, created_at, updated_at) VALUES (?, ?, 1, 1)").run(repositoryId, `/${repositoryId}`);
  database.prepare("INSERT INTO rule_project_repositories (project_id, repository_id, added_at) VALUES (?, ?, 1)").run(`repository:${repositoryId}`, repositoryId);
}

function insertV14Policy(database: Database.Database, rule: PolicyRule, sourceDigest: string | null): void {
  const body = v14Policy(rule);
  database.prepare(
    "INSERT INTO rules (id, kind, mode, revision, enabled, scope, created_at, updated_at, source_digest, format, description, scope_json, checks_json) "
    + "VALUES (?, 'policy', ?, ?, 1, 'repository', 1, 2, ?, ?, ?, ?, ?)",
  ).run(rule.id, rule.mode, rule.revision, sourceDigest, body.format, rule.description ?? null,
    rule.scope === undefined ? null : JSON.stringify(rule.scope), JSON.stringify(body.checks));
}

function insertV14Cosine(database: Database.Database, rule: CosineRule, sourceDigest: string | null): void {
  database.prepare(
    "INSERT INTO rules (id, kind, mode, revision, enabled, scope, created_at, updated_at, source_digest, reference, match_json, sample_json, action_json, repeat_json) "
    + "VALUES (?, 'cosine', ?, 4, 0, 'global', 1, 2, ?, ?, ?, ?, ?, ?)",
  ).run(rule.id, rule.mode, sourceDigest, rule.reference ?? null, JSON.stringify(rule.match), JSON.stringify(rule.sample),
    JSON.stringify(rule.action), rule.repeat === undefined ? null : JSON.stringify(rule.repeat));
}

interface V14CatalogRow {
  ownerKind: "global" | "project";
  ownerId: string;
  kind: "policy" | "cosine";
  rule: Record<string, unknown>;
  sourceDigest: string | null;
  origin: string;
  syncState: string;
  upstreamRuleId: string | null;
}

function insertV14CatalogRule(database: Database.Database, row: V14CatalogRow): void {
  const ruleId = row.rule.id as string;
  database.prepare("INSERT OR IGNORE INTO rule_catalog_identities (rule_id, kind) VALUES (?, ?)").run(ruleId, row.kind);
  database.prepare(
    "INSERT INTO catalog_rules (owner_kind, owner_id, rule_id, kind, revision, enabled, document_json, payload_hash, source_digest, origin, "
    + "sync_state, upstream_rule_id, created_at, updated_at) VALUES (?, ?, ?, ?, 1, 1, ?, ?, ?, ?, ?, ?, 10, 20)",
  ).run(row.ownerKind, row.ownerId, ruleId, row.kind, JSON.stringify({ kind: row.kind, rule: row.rule }),
    digestOf(row.kind, row.rule), row.sourceDigest, row.origin, row.syncState, row.upstreamRuleId);
}

/** Every row of every table but the revision stamp, keyed by table, so two opens can be compared. */
function allRows(path: string): Record<string, unknown[]> {
  const database = new Database(path, { readonly: true });
  try {
    const tables = database.prepare("SELECT name FROM sqlite_master WHERE type = 'table' AND name NOT LIKE 'sqlite_%' AND name <> 'rerail_schema_revision' ORDER BY name")
      .pluck().all() as string[];
    return Object.fromEntries(tables.map(table => [table, database.prepare(`SELECT * FROM \`${table}\` ORDER BY rowid`).all()]));
  } finally {
    database.close();
  }
}

/** Stamp a converted file back to v14 so the next open runs step 14 again. */
function restampV14(path: string): void {
  const database = new Database(path);
  database.exec("UPDATE rerail_schema_revision SET revision = 14; PRAGMA user_version = 14;");
  database.close();
}

function tableNames(database: Database.Database): string[] {
  return database.prepare("SELECT name FROM sqlite_master WHERE type = 'table' ORDER BY name").pluck().all() as string[];
}

const toolCall = { kind: "tool-call", eventId: "s1:tool-call:t1", sessionId: "s1", toolCallId: "t1", toolName: "Read", input: { path: "a.py", limit: 1.5 } };
const toolResult = { kind: "tool-result", eventId: "s1:tool-result:t1", sessionId: "s1", toolCallId: "t1", output: "é \u0001 done" };
/** A payload JSON cannot hold (a bigint), stored only as V8 behind the encoding marker. */
const opaqueCall = { kind: "tool-call", eventId: "s1:tool-call:t2", sessionId: "s1", toolCallId: "t2", toolName: "Count", input: { size: 12n } };
const coverage = { repositoryId: "repo", activeRuleCount: 3, cosineRuleSignature: "[\"cosine\"]", policyRuleSignature: "[\"policy\"]" };
const prompt = { kind: "user-prompt", eventId: "s1:prompt:1", sessionId: "s1", text: "tool-call" };

function insertEvent(database: Database.Database, eventId: string, kind: string, payload: unknown, sessionId = "s1"): void {
  database.prepare("INSERT INTO events (session_id, event_id, kind, observed_at, payload_json, payload_v8) VALUES (?, ?, ?, 5, ?, ?)")
    .run(sessionId, eventId, kind, stableJsonText(payload) ?? JSON.stringify({ encoding: "node:v8" }), serialize(payload));
}

const v14Finding = {
  id: "violation-1", policyId: BUNDLED_POLICY.id, checkId: "clause-a", code: "CODE", mode: "steer", message: "Message",
  path: "a.py", span: { startByte: 0, endByte: 1, startLine: 0, startColumn: 0, endLine: 0, endColumn: 1 }, captures: {}, variables: {},
};
const v15Violation = {
  id: "violation-1", ruleId: BUNDLED_POLICY.id, clauseId: "clause-a", code: "CODE", mode: "steer", message: "Message",
  path: "a.py", span: v14Finding.span, captures: {}, variables: {},
};
const v14Failure = { policyId: BUNDLED_POLICY.id, checkId: "clause-a", code: "source-partial", message: "Partial", path: "b.py", stage: "evaluate" };
const v15Failure = { ruleId: BUNDLED_POLICY.id, clauseId: "clause-a", code: "source-partial", message: "Partial", path: "b.py", stage: "evaluate" };
const v14Provenance = [
  { ruleId: "a", revision: 1, payloadHash: "sha256:a", applicability: "project", projectId: "p", origin: "bundled", syncState: "local-only" },
  { ruleId: "b", revision: 2, payloadHash: "sha256:b", applicability: "global", origin: "local", syncState: "synced", upstreamRuleId: "up-b" },
  { ruleId: "c", revision: 3, payloadHash: "sha256:c", applicability: "project", projectId: "p", origin: "migrated", syncState: "pending" },
  { ruleId: "d", revision: 4, payloadHash: "sha256:d", applicability: "global", origin: "nera", syncState: "synced", upstreamRuleId: "up-d" },
];

/** A checkout database as v14 left it: rules of both kinds, runs with result pages, controls, owners, and events. */
function seedV14Checkout(path: string): { bundledBody: Record<string, unknown>; seededFrom: Record<string, unknown> } {
  const database = createV14Database(path);
  try {
    const bundledBody = v14Policy(BUNDLED_POLICY);
    const seededFrom = v14Policy({ ...EDITED_POLICY, description: "Before the user's edit" });
    insertV14Policy(database, BUNDLED_POLICY, digestOf("policy", bundledBody));
    insertV14Policy(database, EDITED_POLICY, digestOf("policy", seededFrom));
    insertV14Cosine(database, COSINE, digestOf("cosine", { id: COSINE.id, mode: COSINE.mode, reference: COSINE.reference, match: COSINE.match, sample: COSINE.sample, action: COSINE.action, repeat: COSINE.repeat }));
    insertV14Cosine(database, { ...MOCK_COSINE_RULES[1]!, id: "user-cosine" }, null);

    const insertRun = database.prepare(
      "INSERT INTO policy_runs (run_id, session_id, workspace_root, scope, mode, revision, catalog_revision, rule_revisions_json, "
      + "rule_provenance_json, paths_json, started_at, finished_at, status, findings_json, failures_json) "
      + "VALUES (?, 's1', '/workspace', ?, ?, 1, 'sha256:catalog', '[{\"ruleId\":\"a\",\"revision\":1}]', ?, '[\"a.py\"]', ?, ?, ?, ?, ?)",
    );
    insertRun.run("run-steer", "touched-files", "resteer", JSON.stringify(v14Provenance), 1, 2, "completed", JSON.stringify([v14Finding]), JSON.stringify([v14Failure]));
    insertRun.run("run-log", "repository", "log", "[]", 3, null, "running", "[]", "[]");
    const insertPage = database.prepare("INSERT INTO policy_run_result_pages (run_id, page_index, findings_json, failures_json) VALUES ('run-log', ?, ?, ?)");
    insertPage.run(0, JSON.stringify([v14Finding]), "[]");
    insertPage.run(1, "[]", JSON.stringify([v14Failure]));
    database.prepare(
      "INSERT INTO policy_repository_diagnostics (workspace_root, session_id, run_id, status, started_at, finished_at, error) VALUES ('/workspace', 's1', 'run-steer', 'completed', 1, 2, NULL)",
    ).run();
    const insertRequest = database.prepare(
      "INSERT INTO policy_scan_requests (id, session_id, workspace_root, scope, mode, paths_json, status, created_at, started_at, finished_at, run_ids_json, error) "
      + "VALUES (?, 's1', '/workspace', 'repository', ?, '[]', 'completed', ?, 1, 2, ?, NULL)",
    );
    insertRequest.run("request-steer", "resteer", 1, "[\"run-steer\"]");
    insertRequest.run("request-log", "log", 2, "[\"run-log\",\"run-steer\"]");
    const insertSettings = database.prepare("INSERT INTO policy_settings (workspace_root, settings_json, updated_at) VALUES (?, ?, 9)");
    insertSettings.run("/workspace", JSON.stringify({ scope: "touched-files", mode: "resteer" }));
    insertSettings.run("/other", JSON.stringify({ scope: "repository", mode: "log" }));
    database.prepare("INSERT INTO sessions (session_id, host, cwd, started_at, ended_at) VALUES ('s1', 'omp', '/workspace', 1, NULL)").run();
    database.prepare("INSERT INTO policy_session_owners (session_id, delivery, active, process_id, token, updated_at) VALUES ('s1', 'background', 1, 4242, 'token-1', 7)").run();
    insertEvent(database, toolCall.eventId, "tool-call", toolCall);
    insertEvent(database, toolResult.eventId, "tool-result", toolResult);
    insertEvent(database, opaqueCall.eventId, "tool-call", opaqueCall);
    insertEvent(database, "coverage-1", "rule-coverage", coverage);
    insertEvent(database, prompt.eventId, "user-prompt", prompt);
    return { bundledBody, seededFrom };
  } finally {
    database.close();
  }
}

describe("schema v15 upgrade", () => {
  it("converts a v14 checkout database to the Rerail contract", async () => {
    const path = await temporaryDatabase();
    const { seededFrom } = seedV14Checkout(path);

    openRerailDatabase(path).$client.close();

    const database = new Database(path, { readonly: true });
    try {
      expect(database.pragma("user_version", { simple: true })).toBe(RERAIL_SCHEMA_COMPATIBILITY_FLOOR);
      expect(RERAIL_SCHEMA_COMPATIBILITY_FLOOR).toBe(15);
      expect(database.prepare("SELECT revision FROM rerail_schema_revision").pluck().get()).toBe(RERAIL_SCHEMA_VERSION);
      const tables = tableNames(database);
      expect(tables).toEqual(expect.arrayContaining(["audits", "audit_result_pages", "repository_audits", "audit_requests", "audit_settings", "policy_session_owners"]));
      for (const old of ["policy_runs", "policy_run_result_pages", "policy_repository_diagnostics", "policy_scan_requests", "policy_settings", "rules_legacy"]) {
        expect(tables).not.toContain(old);
      }
      expect(database.prepare("SELECT name FROM pragma_table_info('rules')").pluck().all()).toContain("clauses_json");
      expect(database.prepare("SELECT name FROM pragma_table_info('rules')").pluck().all()).not.toContain("checks_json");

      const rules = database.prepare("SELECT id, kind, format, clauses_json, source_digest FROM rules ORDER BY id").all() as {
        id: string; kind: string; format: string | null; clauses_json: string | null; source_digest: string | null;
      }[];
      expect(Object.fromEntries(rules.map(rule => [rule.id, rule.kind]))).toEqual({
        [BUNDLED_POLICY.id]: "file", [EDITED_POLICY.id]: "file", [COSINE.id]: "conversation", "user-cosine": "conversation",
      });
      const bundledRow = rules.find(rule => rule.id === BUNDLED_POLICY.id)!;
      expect(bundledRow.format).toBe("rerail.policy/v1");
      expect(JSON.parse(bundledRow.clauses_json!)).toEqual(BUNDLED_POLICY.clauses);
      expect(bundledRow.source_digest).toBe(digestOf("file", BUNDLED_POLICY));
      expect(rules.find(rule => rule.id === EDITED_POLICY.id)!.source_digest).toBe(digestOf("policy", seededFrom));
      expect(rules.find(rule => rule.id === "user-cosine")!.source_digest).toBeNull();
      expect(rules.find(rule => rule.id === COSINE.id)!.source_digest).toBe(digestOf("conversation", COSINE));

      const audits = database.prepare("SELECT * FROM audits ORDER BY audit_id").all() as Record<string, unknown>[];
      expect(audits.map(audit => [audit.audit_id, audit.mode, audit.status])).toEqual([["run-log", "log", "running"], ["run-steer", "steer", "completed"]]);
      const steer = audits[1]!;
      expect(JSON.parse(steer.violations_json as string)).toEqual([v15Violation]);
      expect(Object.keys(JSON.parse(steer.violations_json as string)[0] as object)).toEqual(Object.keys(v15Violation));
      expect(JSON.parse(steer.failures_json as string)).toEqual([v15Failure]);
      expect(Object.keys(JSON.parse(steer.failures_json as string)[0] as object)).toEqual(Object.keys(v15Failure));
      expect(JSON.parse(steer.rule_provenance_json as string)).toEqual([
        { ruleId: "a", revision: 1, payloadHash: "sha256:a", applicability: "project", projectId: "p", origin: "rerail" },
        { ruleId: "b", revision: 2, payloadHash: "sha256:b", applicability: "global", origin: "user" },
        { ruleId: "c", revision: 3, payloadHash: "sha256:c", applicability: "project", projectId: "p", origin: "user" },
        { ruleId: "d", revision: 4, payloadHash: "sha256:d", applicability: "global", origin: "user" },
      ]);
      expect(steer).toMatchObject({ catalog_revision: "sha256:catalog", rule_revisions_json: "[{\"ruleId\":\"a\",\"revision\":1}]", paths_json: "[\"a.py\"]", started_at: 1, finished_at: 2 });
      expect(database.prepare("SELECT audit_id, page_index, violations_json, failures_json FROM audit_result_pages ORDER BY page_index").all()).toEqual([
        { audit_id: "run-log", page_index: 0, violations_json: JSON.stringify([v15Violation]), failures_json: "[]" },
        { audit_id: "run-log", page_index: 1, violations_json: "[]", failures_json: JSON.stringify([v15Failure]) },
      ]);
      expect(database.prepare("SELECT workspace_root, session_id, audit_id, status FROM repository_audits").all())
        .toEqual([{ workspace_root: "/workspace", session_id: "s1", audit_id: "run-steer", status: "completed" }]);
      expect(database.prepare("SELECT id, mode, audit_ids_json FROM audit_requests ORDER BY created_at").all()).toEqual([
        { id: "request-steer", mode: "steer", audit_ids_json: "[\"run-steer\"]" },
        { id: "request-log", mode: "log", audit_ids_json: "[\"run-log\",\"run-steer\"]" },
      ]);
      expect(database.prepare("SELECT workspace_root, settings_json FROM audit_settings ORDER BY workspace_root").all()).toEqual([
        { workspace_root: "/other", settings_json: "{\"scope\":\"repository\",\"mode\":\"log\"}" },
        { workspace_root: "/workspace", settings_json: "{\"scope\":\"touched-files\",\"mode\":\"steer\"}" },
      ]);
      expect(database.prepare("SELECT * FROM policy_session_owners").all())
        .toEqual([{ session_id: "s1", delivery: "background", active: 1, process_id: 4242, token: "token-1", updated_at: 7 }]);

      const events = database.prepare("SELECT event_id, kind, payload_json, payload_v8 FROM events ORDER BY sequence").all() as {
        event_id: string; kind: string; payload_json: string; payload_v8: Buffer;
      }[];
      expect(events.map(event => [event.event_id, event.kind])).toEqual([
        [toolCall.eventId, "call"],
        [toolResult.eventId, "result"],
        [opaqueCall.eventId, "call"],
        ["coverage-1", "rule-coverage"],
        [prompt.eventId, "user-prompt"],
      ]);
      expect(events[0]!.payload_json).toBe(stableJsonText({ ...toolCall, kind: "call" }));
      expect(deserialize(events[0]!.payload_v8)).toEqual({ ...toolCall, kind: "call" });
      expect(events[1]!.payload_json).toBe(stableJsonText({ ...toolResult, kind: "result" }));
      expect(deserialize(events[1]!.payload_v8)).toEqual({ ...toolResult, kind: "result" });
      expect(events[2]!.payload_json).toBe("{\"encoding\":\"node:v8\"}");
      expect(deserialize(events[2]!.payload_v8)).toEqual({ ...opaqueCall, kind: "call" });
      const renamedCoverage = { repositoryId: "repo", activeRuleCount: 3, conversationRuleSignature: "[\"cosine\"]", fileRuleSignature: "[\"policy\"]" };
      expect(events[3]!.payload_json).toBe(stableJsonText(renamedCoverage));
      expect(Object.keys(deserialize(events[3]!.payload_v8) as object)).toEqual(Object.keys(renamedCoverage));
      expect(deserialize(events[3]!.payload_v8)).toEqual(renamedCoverage);
      expect(events[4]!.payload_json).toBe(stableJsonText(prompt));
    } finally {
      database.close();
    }

    const store = new SqliteRuleStore(path);
    try {
      expect(store.quarantined()).toEqual([]);
      expect(store.get(BUNDLED_POLICY.id)).toMatchObject({ kind: "file", rule: BUNDLED_POLICY, sourceDigest: digestOf("file", BUNDLED_POLICY) });
      expect(store.get(COSINE.id)).toMatchObject({ kind: "conversation", rule: COSINE, revision: 4, enabled: false, scope: "global" });
      const bundled: RuleDocument[] = [{ kind: "file", rule: BUNDLED_POLICY }, { kind: "file", rule: EDITED_POLICY }, { kind: "conversation", rule: COSINE }];
      expect(store.seed(bundled, 50)).toEqual({ inserted: 0, refreshed: 0, kept: 3 });
      // Unedited bundled rows of both kinds keep following the bundle; the edited one keeps the user's edit.
      expect(store.seed([
        { kind: "file", rule: { ...BUNDLED_POLICY, description: "Updated bundle" } },
        { kind: "file", rule: { ...EDITED_POLICY, description: "Updated bundle" } },
        { kind: "conversation", rule: { ...COSINE, reference: "Updated bundle" } },
      ], 60)).toEqual({ inserted: 0, refreshed: 2, kept: 1 });
      expect(store.get(BUNDLED_POLICY.id)?.rule).toMatchObject({ description: "Updated bundle" });
      expect(store.get(EDITED_POLICY.id)?.rule).toMatchObject({ description: EDITED_POLICY.description });
      expect(store.get(COSINE.id)?.rule).toMatchObject({ reference: "Updated bundle" });
    } finally {
      store.close();
    }

    const executions = new SqlitePolicyExecutionStore(path);
    try {
      expect(executions.get("run-steer")).toMatchObject({ auditId: "run-steer", mode: "steer", violations: [v15Violation], failures: [v15Failure] });
      expect(executions.listRepositoryDiagnosticEntries({ auditId: "run-log" })).toBeUndefined();
      expect(executions.listRepositoryDiagnosticEntries({ auditId: "run-steer", kind: "violation" }))
        .toEqual({ entries: [{ kind: "violation", payload: v15Violation }], total: 1 });
      expect(executions.listRepositoryDiagnosticEntries({ auditId: "run-steer", q: "clause-a" })?.total).toBe(2);
      expect(executions.listRepositoryDiagnostics().repositories).toEqual([expect.objectContaining({
        repositoryAudit: expect.objectContaining({ auditId: "run-steer", status: "completed" }),
        audit: expect.objectContaining({ auditId: "run-steer", violationCount: 1, failureCount: 1 }),
      })]);
      expect(executions.ruleActivity(0).get(BUNDLED_POLICY.id)).toEqual({ distinctViolations: 1, sessions: 1, lastSeenAt: 1, audits: 0 });
      expect(executions.listFindingHistory({ since: 0, ruleId: BUNDLED_POLICY.id }).violations)
        .toEqual([{ violation: v15Violation, sessionId: "s1", auditId: "run-steer", at: 1 }]);
    } finally {
      executions.close();
    }
    const controls = new SqliteScanControlStore(path);
    try {
      expect(controls.getWorkspaceSettings("/workspace")).toEqual({ scope: "touched-files", mode: "steer" });
      expect(controls.listScans("s1").map(scan => [scan.id, scan.mode, scan.auditIds])).toEqual([
        ["request-steer", "steer", ["run-steer"]],
        ["request-log", "log", ["run-log", "run-steer"]],
      ]);
    } finally {
      controls.close();
    }
  });

  it("is idempotent: running step 14 again over a converted checkout changes nothing", async () => {
    const path = await temporaryDatabase();
    seedV14Checkout(path);
    openRerailDatabase(path).$client.close();
    const converted = allRows(path);

    openRerailDatabase(path).$client.close();
    expect(allRows(path)).toEqual(converted);
    restampV14(path);
    openRerailDatabase(path).$client.close();
    expect(allRows(path)).toEqual(converted);
  });

  it("converts a v14 machine catalog: kinds, documents, origins, digests, and no sync state", async () => {
    const path = await temporaryDatabase("rules.sqlite");
    const repository = { repositoryId: "repo", workspaceRoot: "/repo" };
    const project = "repository:repo";
    const editedFrom = v14Policy({ ...EDITED_POLICY, description: "Before the user's edit" });
    const legacy = createV14Database(path);
    legacy.exec(
      "INSERT INTO rule_projects (project_id, name, created_at, updated_at) VALUES ('repository:repo', 'repo', 1, 1);"
      + "INSERT INTO rule_repositories (repository_id, last_known_path, created_at, updated_at) VALUES ('repo', '/repo', 1, 1);"
      + "INSERT INTO rule_project_repositories (project_id, repository_id, added_at) VALUES ('repository:repo', 'repo', 1);",
    );
    const bundledBody = v14Policy(BUNDLED_POLICY);
    const cosineBody = { ...COSINE };
    insertV14CatalogRule(legacy, { ownerKind: "project", ownerId: project, kind: "policy", rule: bundledBody, sourceDigest: digestOf("policy", bundledBody), origin: "bundled", syncState: "local-only", upstreamRuleId: null });
    insertV14CatalogRule(legacy, { ownerKind: "project", ownerId: project, kind: "policy", rule: v14Policy(EDITED_POLICY), sourceDigest: digestOf("policy", editedFrom), origin: "local", syncState: "synced", upstreamRuleId: "up-edited" });
    insertV14CatalogRule(legacy, { ownerKind: "global", ownerId: "global", kind: "cosine", rule: cosineBody, sourceDigest: digestOf("cosine", cosineBody), origin: "bundled", syncState: "pending", upstreamRuleId: null });
    insertV14CatalogRule(legacy, { ownerKind: "project", ownerId: project, kind: "cosine", rule: { ...MOCK_COSINE_RULES[1]!, id: "migrated" }, sourceDigest: null, origin: "migrated", syncState: "local-only", upstreamRuleId: null });
    insertV14CatalogRule(legacy, { ownerKind: "global", ownerId: "global", kind: "cosine", rule: { ...MOCK_COSINE_RULES[1]!, id: "from-nera" }, sourceDigest: null, origin: "nera", syncState: "synced", upstreamRuleId: "nera-1" });
    legacy.close();

    const catalog = ScopedRuleCatalog.open(path, repository);
    try {
      expect(catalog.quarantined()).toEqual([]);
      const owned = Object.fromEntries(catalog.listOwned().map(stored => [stored.rule.id, stored]));
      expect(Object.keys(owned).sort()).toEqual([BUNDLED_POLICY.id, EDITED_POLICY.id, COSINE.id, "from-nera", "migrated"].sort());
      expect(owned[BUNDLED_POLICY.id]).toEqual(expect.objectContaining({
        kind: "file", rule: BUNDLED_POLICY, catalogOrigin: "rerail",
        payloadHash: digestOf("file", BUNDLED_POLICY), sourceDigest: digestOf("file", BUNDLED_POLICY),
      }));
      expect(owned[EDITED_POLICY.id]).toEqual(expect.objectContaining({
        kind: "file", rule: EDITED_POLICY, catalogOrigin: "user",
        payloadHash: digestOf("file", EDITED_POLICY), sourceDigest: digestOf("policy", editedFrom),
      }));
      expect(owned[COSINE.id]).toEqual(expect.objectContaining({
        kind: "conversation", rule: COSINE, catalogOrigin: "rerail", applicability: "global",
        payloadHash: digestOf("conversation", COSINE), sourceDigest: digestOf("conversation", COSINE),
      }));
      expect(owned.migrated).toEqual(expect.objectContaining({ kind: "conversation", catalogOrigin: "user", sourceDigest: null }));
      expect(owned["from-nera"]).toEqual(expect.objectContaining({ kind: "conversation", catalogOrigin: "user" }));
      for (const stored of Object.values(owned)) {
        expect(stored).not.toHaveProperty("syncState");
        expect(stored).not.toHaveProperty("upstreamRuleId");
      }
      expect(catalog.ruleSignature("file")).not.toBe("[]");

      // The unedited bundled rule keeps following the bundle; the edited one keeps the user's edit.
      expect(catalog.seed([{ kind: "file", rule: BUNDLED_POLICY }, { kind: "file", rule: EDITED_POLICY }], 100))
        .toEqual({ inserted: 0, refreshed: 0, kept: 2 });
      expect(catalog.seed([
        { kind: "file", rule: { ...BUNDLED_POLICY, description: "Updated bundle" } },
        { kind: "file", rule: { ...EDITED_POLICY, description: "Updated bundle" } },
      ], 200)).toEqual({ inserted: 0, refreshed: 1, kept: 1 });
      expect(catalog.get(BUNDLED_POLICY.id)?.rule).toMatchObject({ description: "Updated bundle" });
      expect(catalog.get(EDITED_POLICY.id)?.rule).toMatchObject({ description: EDITED_POLICY.description });
    } finally {
      catalog.close();
    }

    const database = new Database(path, { readonly: true });
    try {
      const columns = database.prepare("SELECT name FROM pragma_table_info('catalog_rules')").pluck().all();
      expect(columns).not.toContain("sync_state");
      expect(columns).not.toContain("upstream_rule_id");
      expect(database.prepare("SELECT rule_id, kind FROM rule_catalog_identities ORDER BY rule_id").all()).toEqual([
        { rule_id: BUNDLED_POLICY.id, kind: "file" },
        { rule_id: EDITED_POLICY.id, kind: "file" },
        { rule_id: "from-nera", kind: "conversation" },
        { rule_id: COSINE.id, kind: "conversation" },
        { rule_id: "migrated", kind: "conversation" },
      ].sort((left, right) => (left.rule_id < right.rule_id ? -1 : 1)));
      const document = JSON.parse(database.prepare("SELECT document_json FROM catalog_rules WHERE rule_id = ?").pluck().get(EDITED_POLICY.id) as string);
      expect(document).toEqual({ kind: "file", rule: EDITED_POLICY });
      expect(Object.keys(document.rule)).not.toContain("checks");
    } finally {
      database.close();
    }
    const reader = openRerailDatabase(path, { readonly: true });
    try {
      expect(JSON.parse(scopedRuleSignature(reader, "repo", "conversation"))).toHaveLength(3);
    } finally {
      reader.$client.close();
    }
  });

  it("is idempotent over a converted catalog", async () => {
    const path = await temporaryDatabase("rules.sqlite");
    const legacy = createV14Database(path);
    const body = v14Policy(BUNDLED_POLICY);
    insertV14CatalogRule(legacy, { ownerKind: "global", ownerId: "global", kind: "policy", rule: body, sourceDigest: digestOf("policy", body), origin: "bundled", syncState: "synced", upstreamRuleId: "up" });
    legacy.close();
    openRerailDatabase(path).$client.close();
    const converted = allRows(path);
    restampV14(path);
    openRerailDatabase(path).$client.close();
    expect(allRows(path)).toEqual(converted);
  });

  it("brings a v3 database through the frozen v14 steps to v15", async () => {
    const path = await temporaryDatabase();
    const legacy = createV14Database(path);
    legacy.exec(
      "DROP TABLE policy_run_result_pages; DROP TABLE policy_runs; DROP TABLE policy_repository_diagnostics; DROP TABLE rerail_schema_revision;"
      + "DROP INDEX sessions_repository_started; ALTER TABLE sessions DROP COLUMN repository_id; PRAGMA user_version = 3;",
    );
    insertV14Policy(legacy, BUNDLED_POLICY, null);
    legacy.prepare("INSERT INTO sessions (session_id, host, cwd, started_at, ended_at) VALUES ('old', 'omp', '/workspace', 1, 2)").run();
    legacy.close();

    openRerailDatabase(path).$client.close();
    const database = new Database(path, { readonly: true });
    try {
      expect(database.pragma("user_version", { simple: true })).toBe(15);
      const tables = tableNames(database);
      expect(tables).toEqual(expect.arrayContaining(["audits", "audit_result_pages", "repository_audits"]));
      expect(tables).not.toContain("policy_runs");
      expect(database.prepare("SELECT kind, format FROM rules").all()).toEqual([{ kind: "file", format: "rerail.policy/v1" }]);
      expect(database.prepare("SELECT session_id, repository_id FROM sessions").all()).toEqual([{ session_id: "old", repository_id: null }]);
    } finally {
      database.close();
    }
  });

  it("converts a v14 rerail.sqlite that holds the catalog and every repository's history, and keeps it scoped", async () => {
    const home = await temporaryDirectory();
    const path = join(home, "rerail.sqlite");
    const database = createV14Database(path);
    try {
      insertRepositoryProject(database, "repo-a");
      const defineBody = v14Policy(DEFINE_POLICY);
      insertV14CatalogRule(database, { ownerKind: "project", ownerId: "repository:repo-a", kind: "policy", rule: defineBody, sourceDigest: digestOf("policy", defineBody), origin: "bundled", syncState: "local-only", upstreamRuleId: null });
      insertV14CatalogRule(database, { ownerKind: "global", ownerId: "global", kind: "cosine", rule: { ...COSINE }, sourceDigest: null, origin: "local", syncState: "synced", upstreamRuleId: "up-cosine" });
      const insertSession = database.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES (?, 'omp', ?, ?, 1, ?)");
      insertSession.run("s-a", "/a", "repo-a", 9);
      insertSession.run("s-a-live", "/a", "repo-a", null);
      insertSession.run("s-b", "/b", "repo-b", 9);
      const insertRun = database.prepare(
        "INSERT INTO policy_runs (run_id, session_id, workspace_root, scope, mode, revision, rule_provenance_json, paths_json, started_at, finished_at, status, findings_json, failures_json) "
        + "VALUES (?, ?, ?, 'repository', 'log', 1, ?, '[]', ?, 3, 'completed', ?, '[]')",
      );
      insertRun.run("run-a", "s-a", "/a", JSON.stringify(v14Provenance.slice(0, 1)), 2, JSON.stringify([v14Finding]));
      insertRun.run("run-b", "s-b", "/b", "[]", 4, "[]");
      database.prepare("INSERT INTO policy_run_result_pages (run_id, page_index, findings_json, failures_json) VALUES ('run-a', 0, ?, '[]')").run(JSON.stringify([v14Finding]));
      const insertDiagnostic = database.prepare(
        "INSERT INTO policy_repository_diagnostics (workspace_root, session_id, run_id, status, started_at, finished_at, error) VALUES (?, ?, ?, 'completed', 2, 3, NULL)",
      );
      insertDiagnostic.run("/a", "s-a", "run-a");
      insertDiagnostic.run("/b", "s-b", "run-b");
      const insertRequest = database.prepare(
        "INSERT INTO policy_scan_requests (id, session_id, workspace_root, scope, mode, paths_json, status, created_at, started_at, finished_at, run_ids_json, error) "
        + "VALUES (?, ?, ?, 'repository', 'resteer', '[]', 'completed', 1, 2, 3, ?, NULL)",
      );
      insertRequest.run("request-a", "s-a", "/a", "[\"run-a\"]");
      insertRequest.run("request-b", "s-b", "/b", "[\"run-b\"]");
      database.prepare("INSERT INTO policy_session_owners (session_id, delivery, active, process_id, token, updated_at) VALUES ('s-a-live', 'background', 1, 7, 'token', 1)").run();
      insertEvent(database, "s-a:tool-call:t1", "tool-call", { ...toolCall, eventId: "s-a:tool-call:t1", sessionId: "s-a" }, "s-a");
      insertEvent(database, "s-b:tool-result:t1", "tool-result", { ...toolResult, eventId: "s-b:tool-result:t1", sessionId: "s-b" }, "s-b");
    } finally {
      database.close();
    }

    const catalog = ScopedRuleCatalog.open(path, { repositoryId: "repo-a", workspaceRoot: join(home, "a") });
    try {
      expect(catalog.quarantined()).toEqual([]);
      expect(catalog.get(DEFINE_POLICY.id)).toMatchObject({
        kind: "file", rule: DEFINE_POLICY, catalogOrigin: "rerail",
        payloadHash: digestOf("file", DEFINE_POLICY), sourceDigest: digestOf("file", DEFINE_POLICY),
      });
      expect((catalog.get(DEFINE_POLICY.id)?.rule as PolicyRule).clauses[0]).toEqual(DEFINE_CLAUSE);
      expect(catalog.get(COSINE.id)).toMatchObject({ kind: "conversation", rule: COSINE, applicability: "global", catalogOrigin: "user" });
    } finally {
      catalog.close();
    }

    const upgraded = new Database(path, { readonly: true });
    try {
      expect(upgraded.pragma("user_version", { simple: true })).toBe(15);
      expect(upgraded.prepare("SELECT revision FROM rerail_schema_revision").pluck().get()).toBe(RERAIL_SCHEMA_VERSION);
      expect(tableNames(upgraded).filter(name => name.startsWith("policy_"))).toEqual(["policy_session_owners"]);
      expect(upgraded.prepare("SELECT session_id, repository_id FROM sessions ORDER BY session_id").all()).toEqual([
        { session_id: "s-a", repository_id: "repo-a" },
        { session_id: "s-a-live", repository_id: "repo-a" },
        { session_id: "s-b", repository_id: "repo-b" },
      ]);
      expect(upgraded.prepare("SELECT name FROM sqlite_master WHERE type = 'index' AND name = 'sessions_repository_started'").pluck().get())
        .toBe("sessions_repository_started");
      expect(upgraded.prepare("SELECT id, mode, audit_ids_json FROM audit_requests ORDER BY id").all()).toEqual([
        { id: "request-a", mode: "steer", audit_ids_json: "[\"run-a\"]" },
        { id: "request-b", mode: "steer", audit_ids_json: "[\"run-b\"]" },
      ]);
      expect(upgraded.prepare("SELECT event_id, kind FROM events ORDER BY sequence").all()).toEqual([
        { event_id: "s-a:tool-call:t1", kind: "call" },
        { event_id: "s-b:tool-result:t1", kind: "result" },
      ]);
    } finally {
      upgraded.close();
    }

    const reader = new SqlitePolicyExecutionReader(path, { repositoryId: "repo-a" });
    try {
      expect(reader.list()).toMatchObject({ audits: [{ auditId: "run-a", violations: [v15Violation] }], total: 1 });
      expect(reader.listRepositoryDiagnostics().repositories).toEqual([expect.objectContaining({
        workspaceRoot: "/a",
        repositoryAudit: expect.objectContaining({ auditId: "run-a" }),
        audit: expect.objectContaining({ auditId: "run-a", violationCount: 1 }),
      })]);
      expect(reader.listFindingHistory({ since: 0 }).violations).toEqual([{ violation: v15Violation, sessionId: "s-a", auditId: "run-a", at: 2 }]);
    } finally {
      reader.close();
    }

    expect(resetRepositoryHistory(path, "repo-a")).toEqual({ sessions: 1, openSessions: 1 });
    const reset = new Database(path, { readonly: true });
    try {
      expect(reset.prepare("SELECT audit_id FROM audits").pluck().all()).toEqual(["run-b"]);
      expect(reset.prepare("SELECT count(*) FROM audit_result_pages").pluck().get()).toBe(0);
      expect(reset.prepare("SELECT audit_id FROM repository_audits").pluck().all()).toEqual(["run-b"]);
      expect(reset.prepare("SELECT id FROM audit_requests").pluck().all()).toEqual(["request-b"]);
      expect(reset.prepare("SELECT session_id FROM events").pluck().all()).toEqual(["s-b"]);
      expect(reset.prepare("SELECT session_id FROM sessions ORDER BY session_id").pluck().all()).toEqual(["s-a-live", "s-b"]);
      expect(reset.prepare("SELECT session_id FROM policy_session_owners").pluck().all()).toEqual(["s-a-live"]);
      expect(reset.prepare("SELECT count(*) FROM catalog_rules").pluck().get()).toBe(2);
    } finally {
      reset.close();
    }
  });

  it("brings a main v13 database (repository attribution, no rule modes) to v15", async () => {
    const home = await temporaryDirectory();
    const path = join(home, "rerail.sqlite");
    const database = createV14Database(path);
    try {
      // Main stamps compatibility floor 10 beside revision 13, and its rules table has no mode column.
      database.exec(`DROP TABLE rules; ${rulesTableWithoutMode()}; UPDATE rerail_schema_revision SET revision = 13; PRAGMA user_version = 10;`);
      insertRepositoryProject(database, "repo");
      const bundledBody = mainPolicy(BUNDLED_POLICY);
      const defineBody = mainPolicy(DEFINE_POLICY);
      const cosineBody = mainCosine(COSINE);
      insertV14CatalogRule(database, { ownerKind: "project", ownerId: "repository:repo", kind: "policy", rule: bundledBody, sourceDigest: digestOf("policy", bundledBody), origin: "bundled", syncState: "local-only", upstreamRuleId: null });
      insertV14CatalogRule(database, { ownerKind: "project", ownerId: "repository:repo", kind: "policy", rule: defineBody, sourceDigest: null, origin: "local", syncState: "local-only", upstreamRuleId: null });
      insertV14CatalogRule(database, { ownerKind: "global", ownerId: "global", kind: "cosine", rule: cosineBody, sourceDigest: digestOf("cosine", cosineBody), origin: "bundled", syncState: "pending", upstreamRuleId: null });
      database.prepare(
        "INSERT INTO rules (id, kind, revision, enabled, scope, created_at, updated_at, source_digest, format, description, scope_json, checks_json) "
        + "VALUES (?, 'policy', ?, 1, 'repository', 1, 2, ?, 'rerail.policy/v1', ?, ?, ?)",
      ).run(BUNDLED_POLICY.id, BUNDLED_POLICY.revision, digestOf("policy", bundledBody), BUNDLED_POLICY.description ?? null,
        BUNDLED_POLICY.scope === undefined ? null : JSON.stringify(BUNDLED_POLICY.scope), JSON.stringify(bundledBody.checks));
      database.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES ('s1', 'omp', '/repo', 'repo', 1, 2)").run();
      const { mode: _mode, ...unmoded } = v14Finding;
      const findings = JSON.stringify([{ ...unmoded, severity: "warning" }]);
      database.prepare(
        "INSERT INTO policy_runs (run_id, session_id, workspace_root, scope, mode, revision, rule_provenance_json, paths_json, started_at, finished_at, status, findings_json, failures_json) "
        + "VALUES ('run', 's1', '/repo', 'touched-files', 'resteer', 1, '[]', '[]', 1, 2, 'completed', ?, '[]')",
      ).run(findings);
      database.prepare("INSERT INTO policy_run_result_pages (run_id, page_index, findings_json, failures_json) VALUES ('run', 0, ?, '[]')").run(findings);
      insertEvent(database, toolCall.eventId, "tool-call", toolCall);
    } finally {
      database.close();
    }

    const catalog = ScopedRuleCatalog.open(path, { repositoryId: "repo", workspaceRoot: join(home, "repo") });
    try {
      expect(catalog.quarantined()).toEqual([]);
      expect(catalog.get(BUNDLED_POLICY.id)).toMatchObject({
        kind: "file", rule: BUNDLED_POLICY, catalogOrigin: "rerail",
        payloadHash: digestOf("file", BUNDLED_POLICY), sourceDigest: digestOf("file", BUNDLED_POLICY),
      });
      expect(catalog.get(DEFINE_POLICY.id)).toMatchObject({ kind: "file", rule: DEFINE_POLICY, catalogOrigin: "user", sourceDigest: null });
      expect(catalog.get(COSINE.id)).toMatchObject({
        kind: "conversation", rule: COSINE, catalogOrigin: "rerail", sourceDigest: digestOf("conversation", COSINE),
      });
    } finally {
      catalog.close();
    }
    const store = new SqliteRuleStore(path);
    try {
      expect(store.get(BUNDLED_POLICY.id)).toMatchObject({ kind: "file", rule: BUNDLED_POLICY, sourceDigest: digestOf("file", BUNDLED_POLICY) });
    } finally {
      store.close();
    }

    const upgraded = new Database(path, { readonly: true });
    try {
      expect(upgraded.pragma("user_version", { simple: true })).toBe(15);
      expect(upgraded.prepare("SELECT revision FROM rerail_schema_revision").pluck().get()).toBe(RERAIL_SCHEMA_VERSION);
      expect(upgraded.prepare("SELECT name FROM pragma_table_info('catalog_rules')").pluck().all()).not.toContain("sync_state");
      expect(upgraded.prepare("SELECT session_id, repository_id FROM sessions").all()).toEqual([{ session_id: "s1", repository_id: "repo" }]);
      const audits = upgraded.prepare("SELECT audit_id, mode, violations_json FROM audits").all() as Record<string, unknown>[];
      expect(audits.map(audit => ({ ...audit, violations_json: JSON.parse(audit.violations_json as string) })))
        .toEqual([{ audit_id: "run", mode: "steer", violations_json: [v15Violation] }]);
      expect(JSON.parse(upgraded.prepare("SELECT violations_json FROM audit_result_pages").pluck().get() as string)).toEqual([v15Violation]);
      expect(upgraded.prepare("SELECT kind FROM events").pluck().all()).toEqual(["call"]);
    } finally {
      upgraded.close();
    }
  });

  it("converts the copy of a legacy rules.sqlite that rerail.sqlite adopts, and leaves the legacy file as it was", async () => {
    const home = await temporaryDirectory();
    const legacyPath = join(home, "rules.sqlite");
    const legacy = createV14Database(legacyPath);
    insertRepositoryProject(legacy, "repo");
    const body = v14Policy(BUNDLED_POLICY);
    insertV14CatalogRule(legacy, { ownerKind: "project", ownerId: "repository:repo", kind: "policy", rule: body, sourceDigest: digestOf("policy", body), origin: "bundled", syncState: "local-only", upstreamRuleId: null });
    legacy.close();
    const before = allRows(legacyPath);

    const stderr = vi.spyOn(process.stderr, "write").mockImplementation(() => true);
    const catalog = ScopedRuleCatalog.open(join(home, "rerail.sqlite"), { repositoryId: "repo", workspaceRoot: join(home, "repo") });
    const notices = stderr.mock.calls.map(([line]) => String(line));
    stderr.mockRestore();
    expect(notices).toContainEqual(expect.stringContaining(`copied the rule catalog from ${legacyPath}`));
    try {
      expect(catalog.get(BUNDLED_POLICY.id)).toMatchObject({
        kind: "file", rule: BUNDLED_POLICY, catalogOrigin: "rerail", sourceDigest: digestOf("file", BUNDLED_POLICY),
      });
    } finally {
      catalog.close();
    }
    expect(allRows(legacyPath)).toEqual(before);
    const untouched = new Database(legacyPath, { readonly: true });
    try {
      expect(untouched.pragma("user_version", { simple: true })).toBe(14);
    } finally {
      untouched.close();
    }
  });

  it("imports a legacy checkout transcripts.sqlite's rules in the contract shape", async () => {
    const home = await temporaryDirectory();
    const workspaceRoot = join(home, "checkout");
    const checkoutPath = join(workspaceRoot, ".rerail", "transcripts.sqlite");
    const checkout = createV14Database(checkoutPath);
    insertV14Policy(checkout, DEFINE_POLICY, digestOf("policy", v14Policy(DEFINE_POLICY)));
    insertV14Cosine(checkout, { ...MOCK_COSINE_RULES[1]!, id: "checkout-cosine" }, null);
    checkout.close();

    const catalog = ScopedRuleCatalog.open(join(home, "rerail.sqlite"), { repositoryId: "repo", workspaceRoot });
    try {
      expect(catalog.get(DEFINE_POLICY.id)).toMatchObject({
        kind: "file", rule: DEFINE_POLICY, applicability: "project", catalogOrigin: "user", sourceDigest: digestOf("file", DEFINE_POLICY),
      });
      expect((catalog.get(DEFINE_POLICY.id)?.rule as PolicyRule).clauses[0]).toEqual(DEFINE_CLAUSE);
      expect(catalog.get("checkout-cosine")).toMatchObject({ kind: "conversation", applicability: "project", catalogOrigin: "user" });
      expect(catalog.migrateCheckoutRules()).toBe(0);
    } finally {
      catalog.close();
    }
    expect(existsSync(checkoutPath)).toBe(true);
    const upgraded = new Database(checkoutPath, { readonly: true });
    try {
      expect(upgraded.pragma("user_version", { simple: true })).toBe(15);
      expect(upgraded.prepare("SELECT id, kind FROM rules ORDER BY id").all()).toEqual([
        { id: "checkout-cosine", kind: "conversation" },
        { id: DEFINE_POLICY.id, kind: "file" },
      ]);
    } finally {
      upgraded.close();
    }
  });

  it("adds repository attribution to a file an unreleased build stamped v14 without it", async () => {
    const path = await temporaryDatabase("rerail.sqlite");
    openRerailDatabase(path).$client.close();
    const branch = new Database(path);
    branch.exec("DROP INDEX sessions_repository_started; ALTER TABLE sessions DROP COLUMN repository_id; UPDATE rerail_schema_revision SET revision = 14; PRAGMA user_version = 14;");
    branch.prepare("INSERT INTO sessions (session_id, host, cwd, started_at, ended_at) VALUES ('s', 'omp', '/r', 1, NULL)").run();
    branch.close();

    openRerailDatabase(path).$client.close();
    const upgraded = new Database(path, { readonly: true });
    try {
      expect(upgraded.pragma("user_version", { simple: true })).toBe(15);
      expect(upgraded.prepare("SELECT session_id, repository_id FROM sessions").all()).toEqual([{ session_id: "s", repository_id: null }]);
    } finally {
      upgraded.close();
    }
  });


  it("converts audits past one rewrite batch and keeps their result pages, foreign key and cascade", async () => {
    const path = await temporaryDatabase("rerail.sqlite");
    const runs = 450;
    const database = createV14Database(path);
    try {
      database.prepare("INSERT INTO sessions (session_id, host, cwd, repository_id, started_at, ended_at) VALUES ('s1', 'omp', '/a', 'repo', 1, 2)").run();
      const insertRun = database.prepare(
        "INSERT INTO policy_runs (run_id, session_id, workspace_root, scope, mode, revision, rule_provenance_json, paths_json, started_at, finished_at, status, findings_json, failures_json) "
        + "VALUES (?, 's1', '/a', 'repository', 'resteer', 1, '[]', '[]', ?, 3, 'completed', ?, ?)",
      );
      const insertPage = database.prepare("INSERT INTO policy_run_result_pages (run_id, page_index, findings_json, failures_json) VALUES (?, 0, ?, ?)");
      for (let index = 0; index < runs; index += 1) {
        const findings = JSON.stringify([{ ...v14Finding, id: `violation-${index}` }]);
        insertRun.run(`run-${index}`, index, findings, JSON.stringify([v14Failure]));
        insertPage.run(`run-${index}`, findings, JSON.stringify([v14Failure]));
      }
    } finally {
      database.close();
    }

    openRerailDatabase(path).$client.close();
    const upgraded = new Database(path);
    try {
      const count = (query: string): unknown => upgraded.prepare(query).pluck().get();
      expect(count("SELECT count(*) FROM audits")).toBe(runs);
      expect(count("SELECT count(*) FROM audit_result_pages")).toBe(runs);
      expect(count("SELECT count(*) FROM audits WHERE mode <> 'steer' OR violations_json LIKE '%checkId%' OR failures_json LIKE '%policyId%'")).toBe(0);
      expect(count("SELECT count(*) FROM audit_result_pages WHERE violations_json LIKE '%checkId%' OR failures_json LIKE '%policyId%'")).toBe(0);
      expect(JSON.parse(count(`SELECT violations_json FROM audit_result_pages WHERE audit_id = 'run-${runs - 1}'`) as string))
        .toEqual([{ ...v15Violation, id: `violation-${runs - 1}` }]);
      expect(upgraded.prepare("PRAGMA foreign_key_list(audit_result_pages)").all()).toEqual([
        expect.objectContaining({ table: "audits", from: "audit_id", to: "audit_id", on_delete: "CASCADE" }),
      ]);
      expect(upgraded.prepare("PRAGMA foreign_key_check").all()).toEqual([]);
      upgraded.pragma("foreign_keys = ON");
      upgraded.prepare("DELETE FROM audits WHERE audit_id = 'run-0'").run();
      expect(count("SELECT count(*) FROM audit_result_pages")).toBe(runs - 1);
    } finally {
      upgraded.close();
    }
  });

});
