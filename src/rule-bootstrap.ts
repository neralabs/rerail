import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { parseJsonWithoutDuplicateKeys } from "./json-duplicate-keys.js";
import { assertCodeRuleStorable, CodeRuleCompileError } from "./code-rule-compile.js";
import { codeRuleSchema, CODE_RULE_FORMAT } from "./code-rule-schema.js";
import { conversationRuleSchema } from "./rule-schema.js";
import { RULE_APPLICABILITIES, RULE_KINDS, RULE_ORIGINS } from "./schema.js";
import type { BundledRuleDocument, RuleDocument } from "./rule-store.js";
import { ScopedRuleCatalog } from "./scoped-rule-catalog.js";

export const RULES_DUMP_VERSION = 1 as const;
export const RULES_DUMP_FILE = "rules.json";

const PACKAGE_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), "..");

/**
 * `<package>/code-rule-dsl/rules`: core's own rule documents, seeded enabled into every project database. None ship
 * today, so the directory may be absent.
 */
export function bundledRulesDirectory(): string {
  return join(PACKAGE_ROOT, "code-rule-dsl", "rules");
}

/**
 * The rule packs core ships inside its own package (`package.json` `files`) until it loads installed packs: TSLint and
 * complexity.
 * Each is a `packs/<id>/` directory in the pack layout (`docs/pack-format.md`); its `rules/` documents are seeded
 * disabled, and its `pack.json` is read by `src/rule-packs.ts`.
 */
export const CORE_RULE_PACKS = ["tslint", "complexity"] as const;

/** `<package>/packs/<id>` for each pack core ships. */
export function corePackDirectories(): string[] {
  return CORE_RULE_PACKS.map(id => join(PACKAGE_ROOT, "packs", id));
}

/** `rerail rules export` writes beside the database it describes: `$RERAIL_HOME/rules.json`. */
export function defaultRulesDumpPath(databasePath: string): string {
  return join(dirname(resolve(databasePath)), RULES_DUMP_FILE);
}

function parseRuleDocument(value: unknown, label: string): RuleDocument {
  const isCodeRule = typeof value === "object" && value !== null && "format" in value && value.format === CODE_RULE_FORMAT;
  if (isCodeRule) return { kind: "code", rule: codeRuleSchema.parse(value) };
  const conversationRule = conversationRuleSchema.safeParse(value);
  if (conversationRule.success) return { kind: "conversation", rule: conversationRule.data };
  throw new Error(`${label} is neither a ${CODE_RULE_FORMAT} document nor a conversation rule: ${conversationRule.error.issues[0]?.message ?? "invalid"}`);
}

/** The rule document at `path`: a code rule or a conversation rule, JSON without duplicate keys. */
export function readRuleDocument(path: string): RuleDocument {
  return parseRuleDocument(parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path), path);
}

/** Every `*.json` rule document directly in `directory`, sorted by file name; none when it does not exist. */
function readRuleDirectory(directory: string): { path: string; document: RuleDocument }[] {
  if (!existsSync(directory)) return [];
  return readdirSync(directory, { withFileTypes: true })
    .filter(entry => entry.isFile() && entry.name.endsWith(".json"))
    .map(entry => entry.name)
    .sort()
    .map(name => ({ path: join(directory, name), document: readRuleDocument(join(directory, name)) }));
}

/**
 * Core's own rule documents in `directory`, sorted by file name, then each pack's `rules/*.json` (in `packs` order,
 * then by file name) marked `enabledByDefault: false`; ids must be unique across all of them.
 */
export function loadBundledRules(directory = bundledRulesDirectory(), packs: readonly string[] = corePackDirectories()): BundledRuleDocument[] {
  const entries: { path: string; document: BundledRuleDocument }[] = [
    ...readRuleDirectory(directory),
    ...packs
      .flatMap(pack => readRuleDirectory(join(pack, "rules")))
      .map(({ path, document }) => ({ path, document: { ...document, enabledByDefault: false } })),
  ];
  const seen = new Set<string>();
  return entries.map(({ path, document }) => {
    if (seen.has(document.rule.id)) throw new Error(`Duplicate bundled rule id: ${document.rule.id} (${path})`);
    seen.add(document.rule.id);
    return document;
  });
}

const dumpEntrySchema = z.object({
  kind: z.enum(RULE_KINDS),
  rule: z.unknown(),
  enabled: z.boolean(),
  applicability: z.enum(RULE_APPLICABILITIES),
  revision: z.number().int().positive(),
  createdAt: z.number().int(),
  updatedAt: z.number().int(),
  sourceDigest: z.string().nullable(),
  catalogOrigin: z.enum(RULE_ORIGINS),
}).strict();

const rulesDumpSchema = z.object({
  version: z.literal(RULES_DUMP_VERSION),
  exportedAt: z.string(),
  rules: z.array(dumpEntrySchema),
}).strict();

/** Export global and current-project definitions without flattening project overrides. */
export function exportScopedRules(store: ScopedRuleCatalog, path: string, now = new Date()): number {
  const rules = store.listOwned().map(({ origin: _origin, payloadHash: _payloadHash, ...rule }) => rule);
  const dump = { version: RULES_DUMP_VERSION, exportedAt: now.toISOString(), rules };
  const target = resolve(path);
  mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
  const staging = `${target}.${process.pid}.tmp`;
  writeFileSync(staging, `${JSON.stringify(dump, null, 2)}\n`, { mode: 0o600 });
  renameSync(staging, target);
  return rules.length;
}

/**
 * Compile every code rule of a dump in the engine before anything is written, so an import is all or nothing:
 * the error lists each rule the engine rejects, by dump index and id. A rule whose language is not supported yet
 * imports, inactive.
 */
function assertDumpCompiles(path: string, documents: readonly RuleDocument[]): void {
  const problems: string[] = [];
  for (const [index, document] of documents.entries()) {
    if (document.kind !== "code") continue;
    try {
      assertCodeRuleStorable(document.rule);
    } catch (error) {
      if (!(error instanceof CodeRuleCompileError)) throw error;
      problems.push(`  rules[${index}] (${document.rule.id}): ${error.reason}`);
    }
  }
  if (problems.length > 0) {
    throw new Error(`${path}: nothing imported; ${problems.length} code rule(s) do not compile:\n${problems.join("\n")}`);
  }
}

/** Import a dump without flattening applicability. All or nothing. */
export function importScopedRules(store: ScopedRuleCatalog, path: string): number {
  const parsed = rulesDumpSchema.parse(parseJsonWithoutDuplicateKeys(readFileSync(resolve(path), "utf8"), path));
  const entries = parsed.rules.map((entry, index) => {
    const document = parseRuleDocument(entry.rule, `${path} rules[${index}]`);
    if (document.kind !== entry.kind) throw new Error(`${path} rules[${index}] body does not match kind ${entry.kind}`);
    return {
      document,
      options: {
        applicability: entry.applicability,
        enabled: entry.enabled,
        revision: entry.revision,
        createdAt: entry.createdAt,
        now: entry.updatedAt,
        sourceDigest: entry.sourceDigest,
        origin: entry.catalogOrigin,
      },
    };
  });
  assertDumpCompiles(path, entries.map(entry => entry.document));
  return store.restore(entries);
}

