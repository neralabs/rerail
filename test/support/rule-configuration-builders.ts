import { mkdtemp, rm } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach } from "vitest";
import {
  openRerailDatabase,
  type RerailDatabase,
  codeRuleSchema,
  type CodeRule,
  type BundledCatalog,
  type CheckoutContext,
  type RepositoryStatus,
  type RuleMode,
  type RulePack,
  type ConversationRule,
  type RuleDocument,
  repositories,
} from "../../src/engine/index.js";

const directories: string[] = [];
const databases: RerailDatabase[] = [];

afterEach(async () => {
  for (const database of databases.splice(0)) database.$client.close();
  await Promise.all(directories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

export async function temporaryDirectory(): Promise<string> {
  const root = await mkdtemp(join(tmpdir(), "rerail-rule-config-"));
  directories.push(root);
  return root;
}

export async function temporaryDatabase(): Promise<RerailDatabase> {
  const database = openRerailDatabase(join(await temporaryDirectory(), "rerail.sqlite"));
  databases.push(database);
  return database;
}

export function codeRule(id: string, options: { revision?: number; mode?: RuleMode; message?: string } = {}): CodeRule {
  return codeRuleSchema.parse({
    clauses: [{
      id: `${id}:check`,
      violation: { anchor: "witness.span", code: "FORBIDDEN_PRINT", message: options.message ?? "No print" },
      forbid: { regex: { pattern: "\\bprint\\(", source: "file.content" } },
    }],
    format: "rerail.policy/v1",
    id,
    revision: options.revision ?? 1,
    mode: options.mode ?? "block",
  });
}

export function conversation(id: string): ConversationRule {
  return {
    id,
    mode: "steer",
    reference: "Editing a database migration or altering a table schema.",
    match: { operator: "gte", threshold: 0.6 },
    sample: { source: "user-input", boundary: "message" },
    action: { content: "Schema changes need a migration note." },
    repeat: { mode: "once" },
  };
}

/**
 * A small bundle: pack `lint` (`no-console` starts on in Steer, `no-var` starts off in Log), a By Rerail rule
 * `loose`, and a By Rerail conversation rule `talk`. Documents are Block unless the pack says otherwise.
 */
export function testBundle(overrides: { noConsoleRevision?: number } = {}): BundledCatalog {
  const documents: RuleDocument[] = [
    { kind: "file", rule: codeRule("no-console", { revision: overrides.noConsoleRevision ?? 1 }) },
    { kind: "file", rule: codeRule("no-var") },
    { kind: "file", rule: codeRule("loose") },
    { kind: "conversation", rule: conversation("talk") },
  ];
  const pack: RulePack = {
    id: "lint",
    name: "Lint",
    category: "language",
    covers: "TypeScript",
    reason: "TypeScript files",
    match: { any: [{ files: ["**/*.ts"], label: "TypeScript" }] },
    honourSuppressionComments: true,
    rules: [
      { id: "no-console", kind: "file", title: "No console", group: "Lint", mode: "steer", on: true },
      { id: "no-var", kind: "file", title: "No var", group: "Lint", mode: "log", on: false },
    ],
  };
  return {
    documents: new Map(documents.map(document => [document.rule.id, document])),
    packs: new Map([[pack.id, pack]]),
    packOf: new Map(pack.rules.map(rule => [rule.id, pack.id])),
    digest: `test:${overrides.noConsoleRevision ?? 1}`,
  };
}

export const REPOSITORY = "remote:github.com/acme/app";
export const OTHER_REPOSITORY = "remote:github.com/acme/other";

export function addRepository(db: RerailDatabase, repositoryKey: string, configuredAt: number | null = null): void {
  db.insert(repositories).values({ repositoryKey, displayName: repositoryKey, configuredAt, createdAt: 1, updatedAt: 1 }).run();
}

export function checkout(checkoutPath: string, repositoryKey = REPOSITORY): CheckoutContext {
  return { checkoutPath, repositoryKey, cloneId: "clone", git: true, main: true };
}

export const RUNNING: RepositoryStatus = { started: true, setUp: true, startedBy: "repository" };
