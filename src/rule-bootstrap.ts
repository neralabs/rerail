import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { parseJsonWithoutDuplicateKeys } from "./json-duplicate-keys.js";
import { assertPolicyCompiles, PolicyCompileError } from "./policy-kernel-check.js";
import { policyRuleSchema, POLICY_FORMAT } from "./policy-schema.js";
import { cosineRuleSchema } from "./rule-schema.js";
import { RULE_APPLICABILITIES, RULE_KINDS, RULE_ORIGINS, RULE_SYNC_STATES } from "./schema.js";
import type { BundledRuleDocument, RuleDocument } from "./rule-store.js";
import { ScopedRuleCatalog } from "./scoped-rule-catalog.js";

export const RULES_DUMP_VERSION = 1 as const;
export const RULES_DUMP_FILE = "rules.json";

/** `<package>/policies/executable-policy-dsl/rules`: the documents every project database is seeded with. */
export function bundledRulesDirectory(): string {
  return resolve(dirname(fileURLToPath(import.meta.url)), "..", "policies", "executable-policy-dsl", "rules");
}

/** `rerail rules export` writes beside the database it describes: `$RERAIL_HOME/rules.json`. */
export function defaultRulesDumpPath(databasePath: string): string {
  return join(dirname(resolve(databasePath)), RULES_DUMP_FILE);
}

function parseRuleDocument(value: unknown, label: string): RuleDocument {
  const isPolicy = typeof value === "object" && value !== null && "format" in value && value.format === POLICY_FORMAT;
  if (isPolicy) return { kind: "policy", rule: policyRuleSchema.parse(value) };
  const cosine = cosineRuleSchema.safeParse(value);
  if (cosine.success) return { kind: "cosine", rule: cosine.data };
  throw new Error(`${label} is neither a ${POLICY_FORMAT} document nor a cosine rule: ${cosine.error.issues[0]?.message ?? "invalid"}`);
}

/**
 * Subdirectory of the bundled rules holding opt-in packs, one folder per pack (`opt-in/<pack>/*.json`, e.g. the TSLint
 * ports in `opt-in/tslint/`), whose documents are seeded disabled.
 */
export const OPT_IN_RULES_DIRECTORY = "opt-in";

function readRuleDirectory(directory: string): { path: string; document: RuleDocument }[] {
  return readdirSync(directory, { withFileTypes: true })
    .filter(entry => entry.isFile() && entry.name.endsWith(".json"))
    .map(entry => entry.name)
    .sort()
    .map(name => {
      const path = join(directory, name);
      return { path, document: parseRuleDocument(parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path), path) };
    });
}

/**
 * Every opt-in pack's documents, by pack name then file name. Hidden entries (`.DS_Store`, `.hidden-pack/`) and non-JSON
 * files directly in `opt-in/` (a README) are ignored; a `*.json` file there belongs to no pack and is an error.
 */
function readOptInPacks(directory: string): { path: string; document: RuleDocument }[] {
  if (!existsSync(directory)) return [];
  const entries = readdirSync(directory, { withFileTypes: true }).filter(entry => !entry.name.startsWith("."));
  const loose = entries
    .filter(entry => !entry.isDirectory() && entry.name.endsWith(".json"))
    .map(entry => join(directory, entry.name));
  if (loose.length > 0) throw new Error(`Opt-in rules must live in a pack folder (${OPT_IN_RULES_DIRECTORY}/<pack>/*.json): ${loose.join(", ")}`);
  return entries
    .filter(entry => entry.isDirectory())
    .map(entry => entry.name)
    .sort()
    .flatMap(pack => readRuleDirectory(join(directory, pack)));
}

/**
 * Every `*.json` rule document in a directory, sorted by file name, then those in each `opt-in/<pack>/` folder
 * (by pack, then file name) marked `enabledByDefault: false`; ids must be unique across all of them.
 */
export function loadBundledRules(directory = bundledRulesDirectory()): BundledRuleDocument[] {
  const entries: { path: string; document: BundledRuleDocument }[] = [
    ...readRuleDirectory(directory),
    ...readOptInPacks(join(directory, OPT_IN_RULES_DIRECTORY))
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
  syncState: z.enum(RULE_SYNC_STATES),
  upstreamRuleId: z.string().nullable(),
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
 * Compile every policy of a dump in the kernel before anything is written, so an import is all or nothing:
 * the error lists each rule the kernel rejects, by dump index and id.
 */
function assertDumpCompiles(path: string, documents: readonly RuleDocument[]): void {
  const problems: string[] = [];
  for (const [index, document] of documents.entries()) {
    if (document.kind !== "policy") continue;
    try {
      assertPolicyCompiles(document.rule);
    } catch (error) {
      if (!(error instanceof PolicyCompileError)) throw error;
      problems.push(`  rules[${index}] (${document.rule.id}): ${error.reason}`);
    }
  }
  if (problems.length > 0) {
    throw new Error(`${path}: nothing imported; ${problems.length} policy rule(s) do not compile:\n${problems.join("\n")}`);
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
        syncState: entry.syncState,
        upstreamRuleId: entry.upstreamRuleId,
      },
    };
  });
  assertDumpCompiles(path, entries.map(entry => entry.document));
  return store.restore(entries);
}
