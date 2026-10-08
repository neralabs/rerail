import { existsSync, mkdirSync, readdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { parseJsonWithoutDuplicateKeys } from "./json-duplicate-keys.js";
import { assertCodeRuleCompiles, CodeRuleCompileError } from "./code-rule-compile.js";
import { codeRuleSchema, CODE_RULE_FORMAT, type CodeRule } from "./code-rule-schema.js";
import { conversationRuleSchema } from "./rule-schema.js";
import { RULE_APPLICABILITIES, RULE_KINDS, RULE_ORIGINS, RULE_SCOPES } from "./schema.js";
import { SqliteRuleStore, type BundledRuleDocument, type RuleDocument, type SeedReport, type StoredRule } from "./rule-store.js";
import { ScopedRuleCatalog } from "./scoped-rule-catalog.js";

/** The checkout dump version; 2 through 4 are catalog (scoped) dump versions, so the two never share a number. */
export const RULES_DUMP_VERSION = 5 as const;
export const SCOPED_RULES_DUMP_VERSION = 4 as const;
export const RULES_DUMP_FILE = "rules.json";

/** `<package>/code-rule-dsl/rules`: the documents every project database is seeded with. */
export function bundledRulesDirectory(): string {
  return resolve(dirname(fileURLToPath(import.meta.url)), "..", "code-rule-dsl", "rules");
}

/** The dump sits beside the database it describes: `<cwd>/.rerail/rules.json`. */
export function defaultRulesDumpPath(databasePath: string): string {
  return join(dirname(resolve(databasePath)), RULES_DUMP_FILE);
}

function parseRuleDocument(value: unknown, label: string): RuleDocument {
  const isCodeRule = typeof value === "object" && value !== null && "format" in value && value.format === CODE_RULE_FORMAT;
  if (isCodeRule) return { kind: "file", rule: codeRuleSchema.parse(value) };
  const conversationRule = conversationRuleSchema.safeParse(value);
  if (conversationRule.success) return { kind: "conversation", rule: conversationRule.data };
  throw new Error(`${label} is neither a ${CODE_RULE_FORMAT} document nor a conversation rule: ${conversationRule.error.issues[0]?.message ?? "invalid"}`);
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

const storedEntrySchema = z
  .object({
    kind: z.enum(RULE_KINDS),
    rule: z.unknown(),
    enabled: z.boolean(),
    scope: z.enum(RULE_SCOPES),
    revision: z.number().int().positive(),
    createdAt: z.number().int(),
    updatedAt: z.number().int(),
    sourceDigest: z.string().nullable(),
  })
  .strict();

const rulesDumpSchema = z
  .object({
    version: z.literal(RULES_DUMP_VERSION),
    exportedAt: z.string(),
    rules: z.array(storedEntrySchema),
  })
  .strict();

export interface RulesDump {
  version: typeof RULES_DUMP_VERSION;
  exportedAt: string;
  rules: StoredRule[];
}

/** Parse a dump file, validating every rule body against its kind. */
export function readRulesDump(path: string): RulesDump {
  const raw = parseJsonWithoutDuplicateKeys(readFileSync(path, "utf8"), path) as { version?: unknown };
  if (raw.version === SCOPED_RULES_DUMP_VERSION) {
    throw new Error(`${path} is a catalog rules dump; only v${RULES_DUMP_VERSION} checkout dumps can be imported here`);
  }
  if (typeof raw.version === "number" && raw.version < RULES_DUMP_VERSION) {
    throw new Error(`${path} is a v${raw.version} rules dump, which predates the current rule contract; only v${RULES_DUMP_VERSION} checkout dumps can be imported`);
  }
  const parsed = rulesDumpSchema.parse(raw);
  const rules = parsed.rules.map((entry, index): StoredRule => {
    const document = parseRuleDocument(entry.rule, `${path} rules[${index}]`);
    if (document.kind !== entry.kind) throw new Error(`${path} rules[${index}] body does not match kind ${entry.kind}`);
    return { ...entry, ...document };
  });
  return { version: parsed.version, exportedAt: parsed.exportedAt, rules };
}

/** Write the whole rules table to `path` atomically (temp file + rename, mode 0600). */
export function exportRules(store: SqliteRuleStore, path: string, now = new Date()): number {
  const quarantined = store.quarantined();
  if (quarantined.length > 0) throw new Error(`cannot export ${quarantined.length} quarantined rule(s)`);
  const dump: RulesDump = { version: RULES_DUMP_VERSION, exportedAt: now.toISOString(), rules: store.list() };
  const target = resolve(path);
  mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
  const staging = `${target}.${process.pid}.tmp`;
  writeFileSync(staging, `${JSON.stringify(dump, null, 2)}\n`, { mode: 0o600 });
  renameSync(staging, target);
  return dump.rules.length;
}

/** Load a dump into the store; every entry overwrites the row with the same id. */
export function importRules(store: SqliteRuleStore, path: string): number {
  return store.restore(readRulesDump(resolve(path)).rules);
}

const scopedStoredEntrySchema = z.object({
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

const scopedRulesDumpSchema = z.object({
  version: z.literal(SCOPED_RULES_DUMP_VERSION),
  exportedAt: z.string(),
  rules: z.array(scopedStoredEntrySchema),
}).strict();

/** Export global and current-project definitions without flattening project overrides. */
export function exportScopedRules(store: ScopedRuleCatalog, path: string, now = new Date()): number {
  const rules = store.listOwned().map(({ origin: _origin, payloadHash: _payloadHash, ...rule }) => rule);
  const dump = { version: SCOPED_RULES_DUMP_VERSION, exportedAt: now.toISOString(), rules };
  const target = resolve(path);
  mkdirSync(dirname(target), { recursive: true, mode: 0o700 });
  const staging = `${target}.${process.pid}.tmp`;
  writeFileSync(staging, `${JSON.stringify(dump, null, 2)}\n`, { mode: 0o600 });
  renameSync(staging, target);
  return rules.length;
}

/**
 * Compile every code rule of a dump in the engine before anything is written, so an import is all or nothing:
 * the error lists each rule the engine rejects, by dump index and id.
 */
function assertDumpCompiles(path: string, documents: readonly RuleDocument[]): void {
  const problems: string[] = [];
  for (const [index, document] of documents.entries()) {
    if (document.kind !== "file") continue;
    try {
      assertCodeRuleCompiles(document.rule);
    } catch (error) {
      if (!(error instanceof CodeRuleCompileError)) throw error;
      problems.push(`  rules[${index}] (${document.rule.id}): ${error.reason}`);
    }
  }
  if (problems.length > 0) {
    throw new Error(`${path}: nothing imported; ${problems.length} file rule(s) do not compile:\n${problems.join("\n")}`);
  }
}

/** Import a dump without flattening applicability. All or nothing. */
export function importScopedRules(store: ScopedRuleCatalog, path: string): number {
  const source = resolve(path);
  const raw = parseJsonWithoutDuplicateKeys(readFileSync(source, "utf8"), path) as { version?: unknown };
  if (raw.version === RULES_DUMP_VERSION) {
    throw new Error(`${source} is a checkout rules dump; only v${SCOPED_RULES_DUMP_VERSION} catalog dumps can be imported`);
  }
  if (typeof raw.version === "number" && raw.version < SCOPED_RULES_DUMP_VERSION) {
    throw new Error(`${source} is a v${raw.version} rules dump, which predates the current rule contract; only v${SCOPED_RULES_DUMP_VERSION} dumps can be imported`);
  }
  const parsed = scopedRulesDumpSchema.parse(raw);
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

export interface BootstrapRulesOptions {
  bundled?: readonly BundledRuleDocument[];
  /** Read into an empty table before seeding; defaults to the dump beside the database. */
  dumpPath?: string;
  now?: number;
}

export interface BootstrapReport extends SeedReport {
  /** Rows restored from the dump because the table was empty; 0 when nothing was restored. */
  restored: number;
  dumpPath: string;
}

/**
 * `rerail update` leaves a catalog dump (v2 before rule modes, v3 after, v4 since the current rule contract) where a
 * checkout dump would sit. The catalog it came from keeps those rules, upgraded in place, so it is skipped rather
 * than failing the bootstrap. A v1 checkout dump predates the current rule contract and is skipped too.
 */
function restoreRulesDump(store: SqliteRuleStore, dumpPath: string): number {
  const { version } = JSON.parse(readFileSync(dumpPath, "utf8")) as { version?: unknown };
  if (version === RULES_DUMP_VERSION) return importRules(store, dumpPath);
  const catalogDump = typeof version === "number" && version > 1 && version <= SCOPED_RULES_DUMP_VERSION;
  const reason = catalogDump
    ? `it is a v${String(version)} catalog dump, and the catalog keeps its rules`
    : `it is a v${String(version)} rules dump, which predates the current rule contract`;
  process.stderr.write(`rerail: did not restore ${dumpPath}: ${reason}\n`);
  return 0;
}

/**
 * Bring a project database's rules up to date: an empty table is first
 * restored from its dump (so a `rerail reset` or a fresh clone keeps the user's
 * rules), then the bundled documents are seeded without overwriting edits.
 */
export function bootstrapRules(store: SqliteRuleStore, options: BootstrapRulesOptions = {}): BootstrapReport {
  const dumpPath = options.dumpPath ?? defaultRulesDumpPath(store.path);
  const restored = store.count() === 0 && existsSync(dumpPath) ? restoreRulesDump(store, dumpPath) : 0;
  const seeded = store.seed(options.bundled ?? loadBundledRules(), options.now);
  return { ...seeded, restored, dumpPath };
}

export interface ProjectRulesBootstrap {
  report: BootstrapReport;
  /** Enabled code rules, in the shape the engine compiles. */
  codeRules: CodeRule[];
}

/** Open, bootstrap, and close a project database in one step. */
export function bootstrapProjectRules(databasePath: string, options: BootstrapRulesOptions = {}): ProjectRulesBootstrap {
  const store = new SqliteRuleStore(databasePath);
  try {
    const report = bootstrapRules(store, options);
    return { report, codeRules: store.codeRules() };
  } finally {
    store.close();
  }
}
