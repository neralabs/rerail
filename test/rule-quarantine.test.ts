import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { sql } from "drizzle-orm";
import { afterEach, describe, expect, it } from "vitest";
import { openRerailDatabase } from "../src/database.js";
import { codeRuleSchema, type CodeRule } from "../src/code-rule-schema.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { readStoredCodeRule } from "../src/rule-quarantine.js";

const temporaryDirectories: string[] = [];

afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

async function temporaryRoot(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-stored-rules-"));
  temporaryDirectories.push(root);
  return root;
}

/** `MOCK_CODE_RULES[1]` as `rerail seed` writes it. */
const SEED_RULE = {
  format: "rerail.policy/v1",
  id: "mock-lazy-query-shaping",
  revision: 2,
  mode: "steer",
  description: "Route reads must stay inside the lazy query plan; materializing before filtering defeats the ServerPlan budget.",
  scope: { include: ["api/app/routes/**/*.py"], exclude: ["api/app/routes/health.py"] },
  clauses: [
    {
      id: "list-of-query",
      forbid: { ast: { language: "python", rule: { pattern: "list($QUERY)" } } },
      violation: { anchor: "witness.span", code: "RERAIL-MOCK-002", message: "Query materialized before filters were applied." },
    },
  ],
};

/** A code rule with no clauses: valid JSON, rejected by the schema. */
const GARBAGE_RULE = { format: "rerail.policy/v1", id: "garbage", revision: 1, mode: "steer" };

function cleanRule(id: string): CodeRule {
  return codeRuleSchema.parse({
    format: "rerail.policy/v1",
    id,
    revision: 1,
    mode: "steer",
    clauses: [
      {
        id: `${id}:check`,
        violation: { anchor: "witness.span", code: "CLEAN", message: "clean" },
        forbid: { regex: { source: "file.content", pattern: "\\bclean\\(" } },
      },
    ],
  });
}

describe("readStoredCodeRule", () => {
  it("returns a valid body parsed, and names the first schema issue of an invalid one", () => {
    expect(readStoredCodeRule(SEED_RULE)).toEqual({ ok: true, rule: codeRuleSchema.parse(SEED_RULE) });
    expect(readStoredCodeRule(GARBAGE_RULE)).toEqual({ ok: false, reason: expect.stringMatching(/^clauses: /u) });
    expect(readStoredCodeRule("not an object")).toEqual({ ok: false, reason: expect.any(String) });
  });
});

describe("reading stored rows", () => {
  it("skips and diagnoses an invalid row in the per-project rule store", async () => {
    const path = join(await temporaryRoot(), "store.sqlite");
    const store = new SqliteRuleStore(path);
    try {
      store.upsertCodeRule(cleanRule("kept"));
      store.upsertCodeRule(cleanRule("broken"));
    } finally {
      store.close();
    }
    const database = openRerailDatabase(path);
    try {
      database.run(sql`UPDATE rules SET clauses_json = ${JSON.stringify([{ id: "x" }])} WHERE id = ${"broken"}`);
    } finally {
      database.$client.close();
    }
    const reopened = new SqliteRuleStore(path);
    try {
      expect(reopened.list().map(stored => stored.rule.id)).toEqual(["kept"]);
      expect(reopened.get("broken")).toBeUndefined();
      expect(reopened.quarantined()).toMatchObject([{ ruleId: "broken", kind: "code", owner: "repository" }]);
    } finally {
      reopened.close();
    }
  });
});
