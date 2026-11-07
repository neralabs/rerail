import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, realpath, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join, resolve } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { loadRuleCatalog } from "../../../src/engine/catalog/rule-catalog.js";
import { LiveConversationRules } from "../../../src/engine/catalog/conversation-rule-sync.js";
import { openRerailDatabase, type RerailDatabase } from "../../../src/engine/storage/database.js";
import { openCheckoutConversationRules } from "../../../src/engine/code-rule/code-rule-dsl-catalog.js";
import { ConversationRuleRegistry } from "../../../src/engine/catalog/registry.js";
import type { RepositoryKey } from "../../../src/engine/catalog/repository-model.js";
import { setRepositoryStarted } from "../../../src/runtime/index.js";
import { addAttachment, setRuleSetting } from "../../../src/engine/catalog/rule-configuration.js";
import { upsertLibraryRule } from "../../../src/engine/catalog/rule-library.js";
import type { ConversationRule } from "../../../src/engine/catalog/rule-schema.js";
import { MOCK_CODE_RULES } from "../../../src/cli/index.js";
import { setUpTestRepository } from "../../support/repository-rules.js";

const temporaryDirectories: string[] = [];
const connections: RerailDatabase[] = [];
afterEach(async () => {
  for (const connection of connections.splice(0)) connection.$client.close();
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

function rule(id: string, reference: string, overrides: Partial<ConversationRule> = {}): ConversationRule {
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

function matched(registry: ConversationRuleRegistry, text: string, turn = 0): string[] {
  return registry.match({ source: "assistant-text", boundary: "message", text }, { turn }).map(match => match.ruleId);
}

/** A Git checkout whose repository adds `rules`, set up and started. */
async function repository(rules: readonly ConversationRule[]): Promise<{ root: string; databasePath: string; database: RerailDatabase; key: RepositoryKey }> {
  const root = await realpath(await mkdtemp(join(tmpdir(), "rerail-cosine-sync-")));
  temporaryDirectories.push(root);
  execFileSync("git", ["init", "--quiet"], { cwd: root });
  const databasePath = join(root, ".rerail", "rerail.sqlite");
  const key = setUpTestRepository(root, { library: rules.map(rule => ({ kind: "conversation" as const, rule })) }, databasePath);
  const database = openRerailDatabase(databasePath);
  connections.push(database);
  return { root, databasePath, database, key };
}

function off(database: RerailDatabase, key: RepositoryKey, id: string, on: boolean | null): void {
  setRuleSetting(database, { owner: { kind: "repository", repositoryKey: key }, folder: "", targetKind: "rule", targetId: id, on, mode: null });
}

describe("LiveConversationRules", () => {
  // contract (docs/rule-scoping.md): only rules the repository adds run, at their effective `on` and mode.
  it("follows the repository's settings without touching rules the session added", async () => {
    const { root, databasePath, database, key } = await repository([rule("shared", "Guest checkout remains available.")]);
    const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
    const sync = new LiveConversationRules(openCheckoutConversationRules(databasePath, root), registry);
    try {
      sync.prime();
      expect(matched(registry, "Guest checkout remains available.")).toEqual(["shared"]);

      // A rule the session added through the conversation_rules tool is not the sync's to manage.
      registry.upsert(rule("session:only", "Sessions add their own rules."));
      expect(sync.refresh()).toBe(false);

      off(database, key, "shared", false);
      expect(sync.refresh()).toBe(true);
      expect(matched(registry, "Guest checkout remains available.")).toEqual([]);

      off(database, key, "shared", null);
      expect(sync.refresh()).toBe(true);
      expect(matched(registry, "Guest checkout remains available.")).toEqual(["shared"]);

      // A configuration change that leaves the conversation rules alone changes nothing the session sees.
      upsertLibraryRule(database, loadRuleCatalog(), MOCK_CODE_RULES[0]!, "code");
      addAttachment(database, { repositoryKey: key, folder: "", sourceKind: "library_rule", sourceId: MOCK_CODE_RULES[0]!.id });
      expect(sync.refresh()).toBe(false);
      expect(matched(registry, "Sessions add their own rules.")).toEqual(["session:only"]);

      // A stopped repository runs nothing.
      setRepositoryStarted(database, key, false);
      expect(sync.refresh()).toBe(true);
      expect(matched(registry, "Guest checkout remains available.")).toEqual([]);
    } finally {
      sync.close();
    }
  });

  // contract (rule-schema `repeat: once`): deliver at most once per session. Switching a rule off and on is
  // not a new session.
  it("a once rule that already fired stays fired across off and on, and across an edit", async () => {
    const once = rule("db:once", "Guest checkout remains available.", { repeat: { mode: "once" } });
    const { root, databasePath, database, key } = await repository([once]);
    const registry = new ConversationRuleRegistry({ modelPath: resolve("model") });
    const sync = new LiveConversationRules(openCheckoutConversationRules(databasePath, root), registry);
    try {
      sync.prime();
      expect(matched(registry, "Guest checkout remains available.", 2)).toEqual(["db:once"]);
      registry.markDelivered(["db:once"], 2);
      expect(matched(registry, "Guest checkout remains available.", 3)).toEqual([]);

      off(database, key, "db:once", false);
      expect(sync.refresh()).toBe(true);
      off(database, key, "db:once", null);
      expect(sync.refresh()).toBe(true);
      expect(matched(registry, "Guest checkout remains available.", 9)).toEqual([]);

      upsertLibraryRule(database, loadRuleCatalog(), { ...once, action: { content: "edited" } }, "conversation");
      expect(sync.refresh()).toBe(true);
      expect(matched(registry, "Guest checkout remains available.", 10)).toEqual([]);
      // ...but the edit itself did land.
      expect(registry.match({ source: "assistant-text", boundary: "message", text: "Guest checkout remains available." }, { respectRepeat: false })[0]?.action.content).toBe("edited");
    } finally {
      sync.close();
    }
  });

  it("gives a session the conversation rules of the folder it starts in", async () => {
    const { root, databasePath, database, key } = await repository([]);
    await mkdir(join(root, "web", "app"), { recursive: true });
    await mkdir(join(root, "web2"), { recursive: true });
    upsertLibraryRule(database, loadRuleCatalog(), rule("web-only", "Web components stay small."), "conversation");
    addAttachment(database, { repositoryKey: key, folder: "web", sourceKind: "library_rule", sourceId: "web-only" });
    const inside = new ConversationRuleRegistry({ modelPath: resolve("model") });
    const sibling = new ConversationRuleRegistry({ modelPath: resolve("model") });
    const insideSync = new LiveConversationRules(openCheckoutConversationRules(databasePath, join(root, "web", "app")), inside);
    const siblingSync = new LiveConversationRules(openCheckoutConversationRules(databasePath, join(root, "web2")), sibling);
    try {
      insideSync.prime();
      siblingSync.prime();
      expect(matched(inside, "Web components stay small.")).toEqual(["web-only"]);
      expect(matched(sibling, "Web components stay small.")).toEqual([]);
    } finally {
      insideSync.close();
      siblingSync.close();
    }
  });
});
