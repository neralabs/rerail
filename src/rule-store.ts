import { and, asc, count, eq, inArray, ne, or, sql, type SQL } from "drizzle-orm";
import { openRerailDatabase, type RerailDatabase } from "./database.js";
import { readStoredPolicyRule, reportQuarantinedRule, storedDocumentDigest, type QuarantinedRule } from "./rule-quarantine.js";
import { assertPolicyCompiles } from "./policy-kernel-check.js";
import { policyRuleSchema, type PolicyRule } from "./policy-schema.js";
import { RULE_MODES, type RuleMode } from "./rule-mode.js";
import { assertKernelRegexes } from "./rule-regexes.js";
import { parseCosineRule, type CosineRule, type SampleBoundary } from "./rule-schema.js";
import { RULE_KINDS, RULE_SCOPES, rules, type NewRuleRow, type RuleKind, type RuleRow, type RuleScope } from "./schema.js";
import { ruleDocumentDigest } from "./stable-json.js";

interface StoredRuleBase {
  enabled: boolean;
  scope: RuleScope;
  revision: number;
  createdAt: number;
  updatedAt: number;
  /** Digest of the bundled document this row was seeded from; null for a user-created rule. */
  sourceDigest: string | null;
}

export interface StoredPolicyRule extends StoredRuleBase {
  kind: "file";
  rule: PolicyRule;
}

export interface StoredCosineRule extends StoredRuleBase {
  kind: "conversation";
  rule: CosineRule;
}

export type StoredRule = StoredPolicyRule | StoredCosineRule;

/** A rule body with its kind, as bundled with the package or dumped by `rerail rules export`. */
export type RuleDocument = { kind: "file"; rule: PolicyRule } | { kind: "conversation"; rule: CosineRule };

/** A bundled document; `enabledByDefault: false` (opt-in packs) makes the first seed insert it disabled. */
export type BundledRuleDocument = RuleDocument & { readonly enabledByDefault?: boolean };

/** The rule body of a bundled entry, without its seeding metadata. */
export function bundledRuleBody(document: BundledRuleDocument): RuleDocument {
  return document.kind === "file" ? { kind: "file", rule: document.rule } : { kind: "conversation", rule: document.rule };
}

export interface RuleWriteOptions {
  enabled?: boolean;
  scope?: RuleScope;
  /** Conversation rules carry no revision of their own; omit to bump the stored one. File rules take `rule.revision`. */
  revision?: number;
  now?: number;
  /** Bundled provenance; omit to keep what the row already records. */
  sourceDigest?: string | null;
}

export interface SeedReport {
  inserted: number;
  refreshed: number;
  /** Rows left alone: modified by the user, or seeded from the same document already. */
  kept: number;
}


export interface RuleQuery {
  kind?: RuleKind;
  enabled?: boolean;
  scope?: RuleScope;
  /** Case-insensitive substring over id, reference/description, and action content or clause bodies. */
  text?: string;
  /** Conversation `sample.source`; file rules never match. */
  source?: string;
  /** Conversation `sample.boundary`; file rules never match. */
  boundary?: SampleBoundary;
  /** Rule mode; matches either kind. */
  mode?: RuleMode;
}

export type RuleSelection = { ids: readonly string[] } | { filter: RuleQuery };

/** SQLite `LIKE` treats `%` and `_` as wildcards; escape them so user text matches literally. */
function likePattern(text: string): string {
  return `%${text.replace(/[\\%_]/gu, "\\$&")}%`;
}

function ruleConditions(query: RuleQuery): SQL[] {
  const conditions: SQL[] = [];
  if (query.kind !== undefined) {
    if (!RULE_KINDS.includes(query.kind)) throw new TypeError(`unknown rule kind ${String(query.kind)}`);
    conditions.push(eq(rules.kind, query.kind));
  }
  if (query.enabled !== undefined) conditions.push(eq(rules.enabled, query.enabled));
  if (query.scope !== undefined) {
    if (!RULE_SCOPES.includes(query.scope)) throw new TypeError(`unknown rule scope ${String(query.scope)}`);
    conditions.push(eq(rules.scope, query.scope));
  }
  const text = query.text?.trim();
  if (text !== undefined && text.length > 0) {
    const pattern = likePattern(text);
    // LIKE is case-insensitive for ASCII in SQLite; ESCAPE makes the pattern literal.
    const matches = (column: SQL | typeof rules.id) => sql`${column} LIKE ${pattern} ESCAPE '\\'`;
    conditions.push(or(
      matches(rules.id),
      matches(sql`coalesce(${rules.reference}, ${rules.description}, '')`),
      matches(sql`coalesce(json_extract(${rules.action}, '$.content'), ${rules.clauses}, '')`),
    )!);
  }
  if (query.source !== undefined) conditions.push(sql`json_extract(${rules.sample}, '$.source') = ${query.source}`);
  if (query.boundary !== undefined) conditions.push(sql`json_extract(${rules.sample}, '$.boundary') = ${query.boundary}`);
  if (query.mode !== undefined) {
    if (!RULE_MODES.includes(query.mode)) throw new TypeError(`unknown rule mode ${String(query.mode)}`);
    conditions.push(eq(rules.mode, query.mode));
  }
  return conditions;
}

const NULL_POLICY_COLUMNS = {
  format: null,
  description: null,
  policyScope: null,
  clauses: null,
} as const satisfies Partial<NewRuleRow>;

const NULL_COSINE_COLUMNS = {
  reference: null,
  match: null,
  sample: null,
  action: null,
  repeat: null,
} as const satisfies Partial<NewRuleRow>;

/** The rule body exactly as the row stores it, before validation. */
function storedBody(row: RuleRow): unknown {
  if (row.kind === "file") {
    return {
      format: row.format,
      id: row.id,
      revision: row.revision,
      mode: row.mode,
      ...(row.description === null ? {} : { description: row.description }),
      ...(row.policyScope === null ? {} : { scope: row.policyScope }),
      clauses: row.clauses,
    };
  }
  return {
    id: row.id,
    mode: row.mode,
    ...(row.reference === null ? {} : { reference: row.reference }),
    match: row.match,
    sample: row.sample,
    action: row.action,
    ...(row.repeat === null ? {} : { repeat: row.repeat }),
  };
}

type RowRead = { ok: true; stored: StoredRule } | { ok: false; quarantined: QuarantinedRule };

/**
 * Rows are re-validated on read: other processes, and plain SQL, write this table too. A row that
 * fails the current schema is quarantined, never thrown.
 */
function readRuleRow(row: RuleRow): RowRead {
  const base: StoredRuleBase = {
    enabled: row.enabled,
    scope: row.scope,
    revision: row.revision,
    createdAt: row.createdAt,
    updatedAt: row.updatedAt,
    sourceDigest: row.sourceDigest,
  };
  const quarantine = (reason: string): RowRead => ({ ok: false, quarantined: { ruleId: row.id, kind: row.kind, owner: row.scope, reason } });
  if (row.kind === "file") {
    const read = readStoredPolicyRule(storedBody(row));
    if (!read.ok) return quarantine(read.reason);
    return { ok: true, stored: { ...base, kind: "file", rule: read.rule } };
  }
  try {
    return { ok: true, stored: { ...base, kind: "conversation", rule: parseCosineRule(storedBody(row), `rules[${row.id}]`) } };
  } catch (error) {
    return quarantine(error instanceof Error ? error.message : String(error));
  }

}

/**
 * Read/write access to the `rules` table. Every write validates the rule body
 * against its kind's schema before it reaches SQLite, so rows always round-trip.
 */
export class SqliteRuleStore {
  readonly path: string;
  readonly #database: RerailDatabase;

  constructor(path: string, database: RerailDatabase = openRerailDatabase(path)) {
    this.path = path;
    this.#database = database;
  }

  /**
   * Validate and insert or replace a `rerail.policy/v1` document. `rule.revision` is stored as-is. The rule is
   * compiled in the kernel first, so one the kernel rejects is never written.
   */
  upsertPolicy(input: unknown, options: RuleWriteOptions = {}): StoredPolicyRule {
    const rule = policyRuleSchema.parse(input);
    assertPolicyCompiles(rule);
    return this.#writePolicy(rule, options);
  }

  #writePolicy(rule: PolicyRule, options: RuleWriteOptions): StoredPolicyRule {
    const existing = this.#row(rule.id);
    if (existing !== undefined && existing.kind !== "file") {
      throw new Error(`rule ${rule.id} exists with kind ${existing.kind}; remove it before storing a file rule`);
    }
    const now = options.now ?? Date.now();
    this.#upsert({
      id: rule.id,
      kind: "file",
      mode: rule.mode,
      revision: rule.revision,
      enabled: options.enabled ?? existing?.enabled ?? true,
      scope: options.scope ?? existing?.scope ?? "repository",
      createdAt: existing?.createdAt ?? now,
      updatedAt: now,
      sourceDigest: options.sourceDigest === undefined ? existing?.sourceDigest ?? null : options.sourceDigest,
      format: rule.format,
      description: rule.description ?? null,
      policyScope: rule.scope ?? null,
      clauses: rule.clauses,
      ...NULL_COSINE_COLUMNS,
    });
    return this.get(rule.id) as StoredPolicyRule;
  }

  /** Validate and insert or replace a conversation rule. The stored revision bumps unless `options.revision` is given. */
  upsertCosine(input: unknown, options: RuleWriteOptions = {}): StoredCosineRule {
    const rule = parseCosineRule(input);
    assertKernelRegexes(rule);
    const existing = this.#row(rule.id);
    if (existing !== undefined && existing.kind !== "conversation") {
      throw new Error(`rule ${rule.id} exists with kind ${existing.kind}; remove it before storing a conversation rule`);
    }
    const now = options.now ?? Date.now();
    this.#upsert({
      id: rule.id,
      kind: "conversation",
      mode: rule.mode,
      revision: options.revision ?? (existing === undefined ? 1 : existing.revision + 1),
      enabled: options.enabled ?? existing?.enabled ?? true,
      scope: options.scope ?? existing?.scope ?? "repository",
      createdAt: existing?.createdAt ?? now,
      updatedAt: now,
      sourceDigest: options.sourceDigest === undefined ? existing?.sourceDigest ?? null : options.sourceDigest,
      ...NULL_POLICY_COLUMNS,
      reference: rule.reference ?? null,
      match: rule.match,
      sample: rule.sample,
      action: rule.action,
      repeat: rule.repeat ?? null,
    });
    return this.get(rule.id) as StoredCosineRule;
  }

  /** Write one document under either kind; the write options apply to whichever branch runs. */
  upsertDocument(document: RuleDocument, options: RuleWriteOptions = {}): StoredRule {
    return document.kind === "file"
      ? this.upsertPolicy(document.rule, options)
      : this.upsertCosine(document.rule, options);
  }

  /** Bundled and restored documents were validated when they were read; a restore must not fail on a kernel change. */
  #writeDocument(document: RuleDocument, options: RuleWriteOptions): StoredRule {
    return document.kind === "file" ? this.#writePolicy(document.rule, options) : this.upsertCosine(document.rule, options);
  }

  /**
   * Seed bundled documents without clobbering the user's edits. A new id is
   * inserted with its bundled digest; a row whose content still equals the
   * digest it was seeded from follows the bundled document when that changes;
   * a row the user edited (content differs from its seed) or created (no seed
   * digest) is left exactly as it is. A new row takes `enabledByDefault`; after that `enabled` and `scope` are
   * never touched.
   */
  seed(documents: readonly BundledRuleDocument[], now = Date.now()): SeedReport {
    const report: SeedReport = { inserted: 0, refreshed: 0, kept: 0 };
    this.#database.transaction(() => {
      for (const entry of documents) {
        const document = bundledRuleBody(entry);
        const bundled = ruleDocumentDigest(document);
        const row = this.#row(document.rule.id);
        if (row === undefined) {
          const revision = document.kind === "conversation" ? { revision: 1 } : {};
          this.#writeDocument(document, { now, sourceDigest: bundled, enabled: entry.enabledByDefault ?? true, ...revision });
          report.inserted += 1;
          continue;
        }
        const read = readRuleRow(row);
        if (!read.ok) {
          // An untouched bundled row the schema now rejects is replaced by its bundled document; a user's row is kept.
          const untouched = row.sourceDigest !== null && row.kind === document.kind
            && storedDocumentDigest(row.kind, storedBody(row)) === row.sourceDigest;
          if (untouched) {
            this.#writeDocument(document, { now, sourceDigest: bundled });
            report.refreshed += 1;
          } else {
            report.kept += 1;
          }
          continue;
        }
        const existing = read.stored;
        const unchanged = existing.sourceDigest === bundled;
        const modified = existing.sourceDigest === null
          || existing.kind !== document.kind
          || ruleDocumentDigest({ kind: existing.kind, rule: existing.rule } as RuleDocument) !== existing.sourceDigest;
        if (unchanged || modified) {
          report.kept += 1;
          continue;
        }
        this.#writeDocument(document, { now, sourceDigest: bundled });
        report.refreshed += 1;
      }
    });
    return report;
  }

  /** Write rows back exactly as another store exported them (timestamps, provenance, and state included). */
  restore(stored: readonly StoredRule[]): number {
    this.#database.transaction(() => {
      for (const entry of stored) {
        this.#writeDocument({ kind: entry.kind, rule: entry.rule } as RuleDocument, {
          enabled: entry.enabled,
          scope: entry.scope,
          revision: entry.revision,
          now: entry.updatedAt,
          sourceDigest: entry.sourceDigest,
        });
        this.#database.update(rules).set({ createdAt: entry.createdAt }).where(eq(rules.id, entry.rule.id)).run();
      }
    });
    return stored.length;
  }

  count(): number {
    return this.#database.select({ value: count() }).from(rules).get()?.value ?? 0;
  }

  get(id: string): StoredRule | undefined {
    const row = this.#row(id);
    return row === undefined ? undefined : this.#read(row);
  }

  /** Rows matching `query` that validate; the rest are reported (see `quarantined`) and left out. */
  list(query: RuleQuery = {}): StoredRule[] {
    return this.#rows(query).flatMap(row => this.#read(row) ?? []);
  }

  /** Rows that fail validation, and are therefore skipped by every read. */
  quarantined(): QuarantinedRule[] {
    return this.#rows({}).flatMap(row => {
      const read = readRuleRow(row);
      return read.ok ? [] : [read.quarantined];
    });
  }

  /** Enabled conversation rules in the shape `CosineRuleRegistry.replace` accepts. */
  cosineRules(): CosineRule[] {
    return this.list({ kind: "conversation", enabled: true }).flatMap(stored =>
      stored.kind === "conversation" ? [stored.rule] : [],
    );
  }

  /** Ids of every `conversation` row, enabled or not: the ids the database owns. */
  cosineIds(): Set<string> {
    return new Set(
      this.#database.select({ id: rules.id }).from(rules).where(eq(rules.kind, "conversation")).all().map(row => row.id),
    );
  }

  /** Enabled file rules, validated. */
  policyRules(): PolicyRule[] {
    return this.list({ kind: "file", enabled: true }).flatMap(stored =>
      stored.kind === "file" ? [stored.rule] : [],
    );
  }

  /** SQLite's `data_version`: changes whenever another connection commits to this file; a cheap "anything new?" probe. */
  dataVersion(): number {
    const row = this.#database.all<{ data_version: number }>(sql`PRAGMA data_version`)[0];
    return row?.data_version ?? 0;
  }

  /** Flip `enabled` on the named ids or on every rule matching a filter. Returns how many rows changed state. */
  setEnabled(selection: RuleSelection, enabled: boolean, now = Date.now()): number {
    const scope = "ids" in selection ? [inArray(rules.id, [...selection.ids])] : ruleConditions(selection.filter);
    if ("ids" in selection && selection.ids.length === 0) return 0;
    return this.#database
      .update(rules)
      .set({ enabled, updatedAt: now })
      .where(and(...scope, ne(rules.enabled, enabled)))
      .returning({ id: rules.id })
      .all().length;
  }

  remove(ids: readonly string[]): number {
    if (ids.length === 0) return 0;
    return this.#database.delete(rules).where(inArray(rules.id, [...ids])).returning({ id: rules.id }).all().length;
  }

  close(): void {
    this.#database.$client.close();
  }

  #upsert(row: NewRuleRow): void {
    const { id: _id, createdAt: _createdAt, ...update } = row;
    this.#database.insert(rules).values(row).onConflictDoUpdate({ target: rules.id, set: update }).run();
  }

  #rows(query: RuleQuery): RuleRow[] {
    return this.#database.select().from(rules).where(and(...ruleConditions(query))).orderBy(asc(rules.id)).all();
  }

  #read(row: RuleRow): StoredRule | undefined {
    const read = readRuleRow(row);
    if (read.ok) return read.stored;
    reportQuarantinedRule(this.path, read.quarantined);
    return undefined;
  }

  #row(id: string): RuleRow | undefined {
    return this.#database.select().from(rules).where(eq(rules.id, id)).get();
  }
}
