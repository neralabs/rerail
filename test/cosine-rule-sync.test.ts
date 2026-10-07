import { mkdtemp, mkdir, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import Database from "better-sqlite3";
import { afterEach, describe, expect, it } from "vitest";
import { LiveCosineRules } from "../src/cosine-rule-sync.js";
import { CosineRuleRegistry } from "../src/registry.js";
import type { CosineRule } from "../src/rule-schema.js";
import { cosineRuleLoader } from "../src/rule-sources.js";
import { SqliteRuleStore } from "../src/rule-store.js";
import { ScopedRuleCatalog } from "../src/scoped-rule-catalog.js";
import { MOCK_POLICY_RULES } from "../src/seed.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

function rule(id: string, reference: string, overrides: Partial<CosineRule> = {}): CosineRule {
  return {
    id,
    mode: "steer",
    reference,
    match: { operator: "gte", threshold: 0.99 },
    sample: { source: "assistant-text", boundary: "message", minimumTokens: 1 },
    action: { content: `Remember: ${id}` },
    repeat: { mode: "always" },
    ...overrides,
  };
}

function matched(registry: CosineRuleRegistry, text: string, turn = 0): string[] {
  return registry.match({ source: "assistant-text", boundary: "message", text }, { turn }).map(match => match.ruleId);
}

async function project(): Promise<{ root: string; databasePath: string }> {
  const root = await mkdtemp(join(tmpdir(), "rerail-cosine-sync-"));
  temporaryDirectories.push(root);
  return { root, databasePath: join(root, ".rerail", "rerail.sqlite") };
}

describe("LiveCosineRules", () => {
  // contract (docs/rule-scoping.md): the catalog is authoritative; a catalog row replaces a file
  // rule with the same id. A disabled row is still the catalog's word on that id.
  it("a disabled catalog row switches the rule off even when a JSON file carries the same id", async () => {
    const { root, databasePath } = await project();
    await mkdir(join(root, ".omp"), { recursive: true });
    await writeFile(
      join(root, ".omp", "conversation-rules.json"),
      JSON.stringify({
        version: 1,
        rules: [
          rule("shared", "Guest checkout remains available.", { action: { content: "from file" } }),
          rule("later", "Refunds are issued within seven days.", { action: { content: "from file" } }),
        ],
      }),
    );
    const daemon = new SqliteRuleStore(databasePath);
    daemon.upsertCosine(rule("shared", "Guest checkout remains available.", { action: { content: "from database" } }), { now: 1_000 });

    const registry = new CosineRuleRegistry({ modelPath: resolve("model") });
    const sync = new LiveCosineRules(new SqliteRuleStore(databasePath), registry, cosineRuleLoader({}, root, ".omp/conversation-rules.json"));
    try {
      sync.prime();
      expect(registry.match({ source: "assistant-text", boundary: "message", text: "Guest checkout remains available." })[0]?.action.content).toBe("from database");

      // A rule the session added through the conversation_rules tool is not the sync's to manage.
      registry.upsert(rule("session:only", "Sessions add their own rules."));
      expect(sync.refresh()).toBe(false);

      expect(daemon.setEnabled({ ids: ["shared"] }, false, 2_000)).toBe(1);
      expect(sync.refresh()).toBe(true);
      expect(matched(registry, "Guest checkout remains available.")).toEqual([]);

      // Raw SQL from a shell counts too, and re-enabling brings the database copy back, not the file's.
      const shell = new Database(databasePath);
      shell.prepare("UPDATE rules SET enabled = 1, updated_at = 3000 WHERE id = ?").run("shared");
      shell.close();
      expect(sync.refresh()).toBe(true);
      expect(registry.match({ source: "assistant-text", boundary: "message", text: "Guest checkout remains available." })[0]?.action.content).toBe("from database");

      // A row that arrives already disabled takes its id away from the file too: nothing enabled changed, an id did.
      expect(matched(registry, "Refunds are issued within seven days.")).toEqual(["later"]);
      daemon.upsertCosine(rule("later", "Refunds are issued within seven days."), { now: 3_500, enabled: false });
      expect(sync.refresh()).toBe(true);
      expect(matched(registry, "Refunds are issued within seven days.")).toEqual([]);

      // Deleting the row hands the id back to the file.
      expect(daemon.remove(["shared"])).toBe(1);
      expect(sync.refresh()).toBe(true);
      expect(registry.match({ source: "assistant-text", boundary: "message", text: "Guest checkout remains available." })[0]?.action.content).toBe("from file");

      // A commit that leaves the cosine rows alone is not a change, and touches nothing the session owns.
      daemon.upsertPolicy(MOCK_POLICY_RULES[0], { now: 5_000 });
      expect(sync.refresh()).toBe(false);
      expect(matched(registry, "Sessions add their own rules.")).toEqual(["session:only"]);
    } finally {
      sync.close();
      daemon.close();
    }
  });

  // contract (rule-schema `repeat: once`): deliver at most once per session. Switching a rule off and on is
  // not a new session.
  it("a once rule that already fired stays fired across disable and re-enable, and across an edit", async () => {
    const { databasePath } = await project();
    const daemon = new SqliteRuleStore(databasePath);
    daemon.upsertCosine(rule("db:once", "Guest checkout remains available.", { repeat: { mode: "once" } }), { now: 1_000 });
    const registry = new CosineRuleRegistry({ modelPath: resolve("model") });
    const sync = new LiveCosineRules(new SqliteRuleStore(databasePath), registry, database => database.enabled);
    try {
      sync.prime();
      expect(matched(registry, "Guest checkout remains available.", 2)).toEqual(["db:once"]);
      registry.markDelivered(["db:once"], 2);
      expect(matched(registry, "Guest checkout remains available.", 3)).toEqual([]);

      daemon.setEnabled({ ids: ["db:once"] }, false, 2_000);
      expect(sync.refresh()).toBe(true);
      daemon.setEnabled({ ids: ["db:once"] }, true, 3_000);
      expect(sync.refresh()).toBe(true);
      expect(matched(registry, "Guest checkout remains available.", 9)).toEqual([]);

      daemon.upsertCosine(rule("db:once", "Guest checkout remains available.", { repeat: { mode: "once" }, action: { content: "edited" } }), { now: 4_000 });
      expect(sync.refresh()).toBe(true);
      expect(matched(registry, "Guest checkout remains available.", 10)).toEqual([]);
      // ...but the edit itself did land.
      expect(registry.match({ source: "assistant-text", boundary: "message", text: "Guest checkout remains available." }, { respectRepeat: false })[0]?.action.content).toBe("edited");
    } finally {
      sync.close();
      daemon.close();
    }
  });

  it("refreshes a running repository when another project member edits the shared catalog", async () => {
    const { root } = await project();
    const path = join(root, "rules.sqlite");
    const editor = ScopedRuleCatalog.open(path, { repositoryId: "repo-a", workspaceRoot: "/repo/a" });
    editor.assignRepository("project-shared", "Shared", "repo-a", 1_000);
    editor.assignRepository("project-shared", "Shared", "repo-b", 1_001);
    editor.upsertCosine(rule("shared", "Guest checkout remains available."), { applicability: "project", now: 1_100 });
    const registry = new CosineRuleRegistry({ modelPath: resolve("model") });
    const sync = new LiveCosineRules(
      ScopedRuleCatalog.open(path, { repositoryId: "repo-b", workspaceRoot: "/repo/b" }),
      registry,
      database => database.enabled,
    );
    try {
      sync.prime();
      editor.upsertCosine(
        rule("shared", "Guest checkout remains available.", { action: { content: "edited elsewhere" } }),
        { applicability: "project", now: 2_000 },
      );
      expect(sync.refresh()).toBe(true);
      expect(registry.match(
        { source: "assistant-text", boundary: "message", text: "Guest checkout remains available." },
        { respectRepeat: false },
      )[0]?.action.content).toBe("edited elsewhere");
    } finally {
      sync.close();
      editor.close();
    }
  });

  it("refreshes after a write through the same catalog connection", async () => {
    const { root } = await project();
    const catalog = ScopedRuleCatalog.open(join(root, "rules.sqlite"), {
      repositoryId: "same-process",
      workspaceRoot: root,
    });
    const registry = new CosineRuleRegistry({ modelPath: resolve("model") });
    const sync = new LiveCosineRules(catalog, registry, database => database.enabled);
    try {
      sync.prime();
      catalog.upsertCosine(rule("same-connection", "Guest checkout remains available."), { now: 1_000 });

      expect(sync.refresh()).toBe(true);
      expect(matched(registry, "Guest checkout remains available.")).toEqual(["same-connection"]);
    } finally {
      sync.close();
    }
  });
});
