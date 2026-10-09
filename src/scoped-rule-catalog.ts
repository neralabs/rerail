import { randomUUID } from "node:crypto";
import { chmodSync, existsSync, linkSync, mkdirSync, readFileSync, realpathSync, unlinkSync, writeFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import { and, asc, eq, inArray, sql } from "drizzle-orm";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "./database.js";
import { readStoredCodeRule, reportQuarantinedRule, storedDocumentDigest, type QuarantinedRule } from "./rule-quarantine.js";
import { storableCodeRule } from "./code-rule-compile.js";
import type { CodeRule } from "./code-rule-schema.js";
import { assertEngineRegexes } from "./rule-regexes.js";
import { parseConversationRule, type ConversationRule } from "./rule-schema.js";
import {
  catalogRules,
  ruleCatalogIdentities,
  ruleProjectRepositories,
  ruleProjects,
  ruleRepositories,
  type CatalogRuleRow,
  type RuleApplicability,
  type RuleKind,
  type RuleOrigin,
} from "./schema.js";
import {
  bundledRuleBody,
  type BundledRuleDocument,
  type RuleDocument,
  type RuleQuery,
  type SeedReport,
} from "./rule-store.js";
import { repositoryIdentityLocation } from "./repository-identity.js";
import { ruleDocumentDigest } from "./stable-json.js";

const GLOBAL_OWNER = "global";

export class QuarantinedGlobalRuleError extends Error {
  constructor(ruleId: string) {
    super(`cannot toggle quarantined global rule ${ruleId}`);
    this.name = "QuarantinedGlobalRuleError";
  }
}

function errorCode(error: unknown): string | undefined {
  return typeof error === "object" && error !== null && "code" in error && typeof error.code === "string"
    ? error.code
    : undefined;
}

/** Publish complete identity bytes with an atomic no-clobber link. */
function readOrCreateIdentity(identityPath: string, candidate: string): string {
  try {
    const existing = readFileSync(identityPath, "utf8").trim();
    if (existing.length === 0) throw new Error(`empty Rerail repository identity at ${identityPath}`);
    return existing;
  } catch (error) {
    if (errorCode(error) !== "ENOENT") throw error;
  }
  const temporaryPath = `${identityPath}.${process.pid}.${randomUUID()}.tmp`;
  writeFileSync(temporaryPath, `${candidate}\n`, { encoding: "utf8", flag: "wx", mode: 0o600 });
  try {
    try {
      linkSync(temporaryPath, identityPath);
    } catch (error) {
      if (errorCode(error) !== "EEXIST") throw error;
    }
  } finally {
    unlinkSync(temporaryPath);
  }
  const repositoryId = readFileSync(identityPath, "utf8").trim();
  if (repositoryId.length === 0) throw new Error(`empty Rerail repository identity at ${identityPath}`);
  chmodSync(identityPath, 0o600);
  return repositoryId;
}

/** The canonical location of `path`: its deepest existing ancestor resolved through symlinks, plus the part not yet created. */
export function physicalPath(path: string): string {
  let existing = resolve(path);
  const missing: string[] = [];
  while (!existsSync(existing)) {
    const parent = dirname(existing);
    if (parent === existing) break;
    missing.unshift(basename(existing));
    existing = parent;
  }
  return join(realpathSync(existing), ...missing);
}

export interface RepositoryContext {
  repositoryId: string;
  workspaceRoot: string;
}

/**
 * Resolve a checkout to the repository id stored atomically in Git's common
 * metadata directory. Linked worktrees share it; clones and forks receive a
 * new id. Non-Git workspaces use the same file contract under `.rerail`.
 */
export function resolveRepositoryContext(workspace: string): RepositoryContext {
  const location = repositoryIdentityLocation(workspace);
  if (!location.git) mkdirSync(dirname(location.identityPath), { recursive: true, mode: 0o700 });
  const repositoryId = readOrCreateIdentity(location.identityPath, location.git ? `git:${randomUUID()}` : randomUUID());
  return { repositoryId, workspaceRoot: location.workspaceRoot };
}

/** The catalog for `workspaceRoot`'s repository in the Rerail database. */
export function openScopedRuleCatalog(
  workspaceRoot: string,
  databasePath: string = defaultRerailDatabasePath(),
): ScopedRuleCatalog {
  return ScopedRuleCatalog.open(databasePath, resolveRepositoryContext(workspaceRoot));
}

export interface ScopedRuleWriteOptions {
  applicability?: RuleApplicability;
  enabled?: boolean;
  createdAt?: number;
  revision?: number;
  now?: number;
  sourceDigest?: string | null;
  origin?: RuleOrigin;
}

export type ScopedStoredRule = RuleDocument & {
  enabled: boolean;
  applicability: RuleApplicability;
  origin: { kind: "global" } | { kind: "project"; projectId: string };
  catalogOrigin: RuleOrigin;
  revision: number;
  createdAt: number;
  updatedAt: number;
  sourceDigest: string | null;
  payloadHash: string;
};

export interface RuleCatalogContext extends RepositoryContext {
  projectId: string;
  projectName: string;
}

export interface ScopedRuleQuery extends Omit<RuleQuery, "scope"> {
  applicability?: RuleApplicability;
}

export interface ScopedRuleRestore {
  document: RuleDocument;
  options?: ScopedRuleWriteOptions;
}

export type ScopedRuleSelection = { ids: readonly string[] } | { filter: ScopedRuleQuery };

function compareRuleIds(left: ScopedStoredRule, right: ScopedStoredRule): number {
  if (left.rule.id === right.rule.id) return 0;
  return left.rule.id < right.rule.id ? -1 : 1;
}

function matchesCommonRuleQuery(stored: ScopedStoredRule, query: ScopedRuleQuery): boolean {
  if (query.applicability !== undefined && stored.applicability !== query.applicability) return false;
  if (query.kind !== undefined && stored.kind !== query.kind) return false;
  if (query.enabled !== undefined && stored.enabled !== query.enabled) return false;
  if (query.mode !== undefined && stored.rule.mode !== query.mode) return false;
  const text = query.text?.trim().toLocaleLowerCase("en-US");
  return text === undefined || text.length === 0 || JSON.stringify(stored.rule).toLocaleLowerCase("en-US").includes(text);
}

function matchesRuleQuery(stored: ScopedStoredRule, query: ScopedRuleQuery): boolean {
  if (!matchesCommonRuleQuery(stored, query)) return false;
  if (stored.kind !== "conversation") {
    return query.source === undefined && query.boundary === undefined;
  }
  return (query.source === undefined || stored.rule.sample.source === query.source)
    && (query.boundary === undefined || stored.rule.sample.boundary === query.boundary);
}
/** Compact effective-set identity for passive coverage readers; no rule bodies or writes. */
export function scopedRuleSignature(database: RerailDatabase, repositoryId: string, kind: RuleKind): string {
  return database.all<{ signature: string }>(sql`
    WITH project AS (
      SELECT project_id FROM rule_project_repositories WHERE repository_id = ${repositoryId}
    ), configured AS (
      SELECT rule_id, kind, enabled, revision, payload_hash, owner_kind, owner_id, origin
      FROM catalog_rules AS candidate
      WHERE kind = ${kind} AND (
        owner_id = (SELECT project_id FROM project)
        OR (owner_id = 'global' AND NOT EXISTS (
          SELECT 1 FROM catalog_rules AS override
          WHERE override.owner_id = (SELECT project_id FROM project) AND override.rule_id = candidate.rule_id
        ))
      )
      ORDER BY rule_id
    )
    SELECT coalesce(json_group_array(json_array(rule_id, kind, enabled, revision, payload_hash, owner_kind, owner_id, origin)), '[]') AS signature
    FROM configured
  `)[0]!.signature;
}

/**
 * Machine-authoritative rule catalog. Repository membership and precedence are
 * resolved behind this interface; callers see one effective rule per logical id.
 */
export class ScopedRuleCatalog {
  readonly path: string;
  readonly #database: RerailDatabase;
  readonly #repository: RepositoryContext;
  #closed = false;
  #localRevision = 0;

  static open(path: string, repository: RepositoryContext, database: RerailDatabase = openRerailDatabase(path)): ScopedRuleCatalog {
    return new ScopedRuleCatalog(path, repository, database, true);
  }

  /**
   * A read-only view for status readers (`/rerail`): it never takes the write lock, imports legacy
   * rules, or records the repository. Undefined while the repository has no project yet, which only
   * a writer creates; its writes fail, as the connection is read-only.
   */
  static openReadOnly(path: string, repository: RepositoryContext): ScopedRuleCatalog | undefined {
    const catalog = new ScopedRuleCatalog(path, repository, openRerailDatabase(path, { readonly: true }), false);
    try {
      if (catalog.#project() !== undefined) return catalog;
    } catch (error) {
      catalog.close();
      throw error;
    }
    catalog.close();
    return undefined;
  }

  private constructor(path: string, repository: RepositoryContext, database: RerailDatabase, writable: boolean) {
    this.path = resolve(path);

    this.#database = database;
    this.#repository = { repositoryId: repository.repositoryId, workspaceRoot: resolve(repository.workspaceRoot) };
    if (!writable) return;
    this.#ensureRepository(Date.now());
    if (this.#project() === undefined) {
      this.assignRepository(`repository:${repository.repositoryId}`, basename(this.#repository.workspaceRoot), repository.repositoryId);
    }
  }

  context(): RuleCatalogContext {
    const project = this.#project();
    if (project === undefined) throw new Error(`repository ${this.#repository.repositoryId} has no project`);
    return { ...this.#repository, projectId: project.projectId, projectName: project.name };
  }

  assignRepository(projectId: string, projectName: string, repositoryId: string, now = Date.now()): void {
    if (projectId === GLOBAL_OWNER) throw new Error("project id global is reserved");
    this.#database.transaction(() => {
      this.#database.insert(ruleProjects).values({ projectId, name: projectName, createdAt: now, updatedAt: now })
        .onConflictDoUpdate({ target: ruleProjects.projectId, set: { name: projectName, updatedAt: now } }).run();
      this.#database.insert(ruleRepositories).values({
        repositoryId,
        lastKnownPath: repositoryId === this.#repository.repositoryId ? this.#repository.workspaceRoot : "",
        createdAt: now,
        updatedAt: now,
      }).onConflictDoNothing().run();
      this.#database.insert(ruleProjectRepositories).values({ projectId, repositoryId, addedAt: now })
        .onConflictDoUpdate({ target: ruleProjectRepositories.repositoryId, set: { projectId, addedAt: now } }).run();
    });
    this.#localRevision += 1;
  }
  removeRepository(projectId: string, repositoryId: string, now = Date.now()): boolean {
    const removed = this.#database.transaction(() => {
      const didRemove = this.#database.delete(ruleProjectRepositories)
        .where(and(
          eq(ruleProjectRepositories.projectId, projectId),
          eq(ruleProjectRepositories.repositoryId, repositoryId),
        ))
        .returning({ repositoryId: ruleProjectRepositories.repositoryId }).get() !== undefined;
      if (didRemove) {
        const repository = this.#database.select({ path: ruleRepositories.lastKnownPath }).from(ruleRepositories)
          .where(eq(ruleRepositories.repositoryId, repositoryId)).get();
        const name = repository?.path === undefined || repository.path.length === 0
          ? repositoryId
          : basename(repository.path);
        this.assignRepository(`repository:${repositoryId}`, name, repositoryId, now);
      }
      return didRemove;
    });
    if (removed) this.#localRevision += 1;
    return removed;
  }

  /** Validate against the schema, then compile in the engine, before storing; a rule the engine rejects is never written. */
  upsertCodeRule(input: unknown, options: ScopedRuleWriteOptions = {}): ScopedStoredRule {
    const { rule } = storableCodeRule(input);
    return this.#upsert({ kind: "file", rule }, { ...options, revision: rule.revision });
  }

  upsertConversationRule(input: unknown, options: ScopedRuleWriteOptions = {}): ScopedStoredRule {
    const rule = parseConversationRule(input);
    return this.#upsert({ kind: "conversation", rule }, options);
  }

  upsertDocument(document: RuleDocument, options: ScopedRuleWriteOptions = {}): ScopedStoredRule {
    return document.kind === "file"
      ? this.upsertCodeRule(document.rule, options)
      : this.upsertConversationRule(document.rule, options);
  }

  get(id: string): ScopedStoredRule | undefined {
    return this.#effectiveRows().find(row => row.rule.id === id);
  }

  list(query: ScopedRuleQuery = {}): ScopedStoredRule[] {
    const candidates = query.applicability === undefined
      ? this.#effectiveRows()
      : this.listOwned().filter(rule => rule.applicability === query.applicability);
    return candidates.filter(rule => matchesRuleQuery(rule, query));
  }

  /** Restore a fully validated dump in one transaction. */
  restore(entries: readonly ScopedRuleRestore[]): number {
    return this.#database.transaction(() => {
      for (const entry of entries) this.#upsert(entry.document, entry.options ?? {});
      return entries.length;
    }, { behavior: "immediate" });
  }

  /** Definitions owned by global and the current project, including shadowed global rows. */
  listOwned(): ScopedStoredRule[] {
    return this.#ownedRows().flatMap(row => this.#read(row) ?? []);
  }

  /** Global and current-project rows that fail validation; every read skips them. */
  quarantined(): QuarantinedRule[] {
    return this.#ownedRows().flatMap(row => {
      const read = this.#stored(row);
      return read.ok ? [] : [read.quarantined];
    });
  }

  conversationRules(): ConversationRule[] {
    return this.list({ kind: "conversation", enabled: true }).flatMap(stored => stored.kind === "conversation" ? [stored.rule] : []);
  }

  conversationRuleIds(): Set<string> {
    return new Set(this.list({ kind: "conversation" }).map(stored => stored.rule.id));
  }

  codeRules(): CodeRule[] {
    return this.list({ kind: "file", enabled: true }).flatMap(stored => stored.kind === "file" ? [stored.rule] : []);
  }

  setEnabled(selection: ScopedRuleSelection, enabled: boolean, now = Date.now()): number {
    if ("ids" in selection) {
      const selected = this.#effectiveOwnedRows().filter(row => selection.ids.includes(row.ruleId));
      const changed = this.#database.transaction(() => {
        let count = 0;
        for (const row of selected) {
          if (row.enabled === enabled) continue;
          if (row.ownerKind === "global") {
            const read = this.#stored(row);
            if (!read.ok) throw new QuarantinedGlobalRuleError(row.ruleId);
            this.#upsert({ kind: read.stored.kind, rule: read.stored.rule } as RuleDocument, {
              applicability: "project",
              enabled,
              revision: read.stored.revision,
              now,
              origin: "user",
            });
            count += 1;
            continue;
          }
          this.#database.update(catalogRules).set({ enabled, updatedAt: now }).where(and(
            eq(catalogRules.ownerKind, row.ownerKind),
            eq(catalogRules.ownerId, row.ownerId),
            eq(catalogRules.ruleId, row.ruleId),
          )).run();
          count += 1;
        }
        return count;
      }, { behavior: "immediate" });
      if (changed > 0) this.#localRevision += 1;
      return changed;
    }
    const selected = this.list(selection.filter);
    const updateGlobal = selection.filter.applicability === "global";
    let changed = 0;
    this.#database.transaction(() => {
      for (const stored of selected) {
        if (stored.enabled === enabled) continue;
        if (stored.applicability === "global" && !updateGlobal) {
          this.#upsert({ kind: stored.kind, rule: stored.rule } as RuleDocument, {
            applicability: "project",
            enabled,
            revision: stored.revision,
            now,
            origin: "user",
          });
        } else {
          const ownerId = stored.origin.kind === "global" ? GLOBAL_OWNER : stored.origin.projectId;
          this.#database.update(catalogRules).set({ enabled, updatedAt: now }).where(and(
            eq(catalogRules.ownerKind, stored.applicability),
            eq(catalogRules.ownerId, ownerId),
            eq(catalogRules.ruleId, stored.rule.id),
          )).run();
        }
        changed += 1;
      }
    }, { behavior: "immediate" });
    if (changed > 0) this.#localRevision += 1;
    return changed;
  }

  remove(ids: readonly string[], applicability?: RuleApplicability): number {
    if (ids.length === 0) return 0;
    const selected = (applicability === undefined
      ? this.#effectiveOwnedRows()
      : this.#ownedRows().filter(row => row.ownerKind === applicability))
      .filter(row => ids.includes(row.ruleId));
    const removed = this.#database.transaction(() => {
      let count = 0;
      for (const row of selected) {
        count += this.#database.delete(catalogRules).where(and(
          eq(catalogRules.ownerKind, row.ownerKind),
          eq(catalogRules.ownerId, row.ownerId),
          eq(catalogRules.ruleId, row.ruleId),
        )).returning({ id: catalogRules.ruleId }).all().length;
        const remaining = this.#database.select({ id: catalogRules.ruleId }).from(catalogRules)
          .where(eq(catalogRules.ruleId, row.ruleId)).get();
        if (remaining === undefined) {
          this.#database.delete(ruleCatalogIdentities).where(eq(ruleCatalogIdentities.ruleId, row.ruleId)).run();
        }
      }
      return count;
    });
    if (removed > 0) this.#localRevision += 1;
    return removed;
  }

  /** Seed bundled documents as project rules; new rows take `enabledByDefault`, existing rows keep `enabled`. */
  seed(documents: readonly BundledRuleDocument[], now = Date.now()): SeedReport {
    const report: SeedReport = { inserted: 0, refreshed: 0, kept: 0 };
    for (const entry of documents) {
      const document = bundledRuleBody(entry);
      const row = this.#ownedRow("project", this.context().projectId, document.rule.id);
      const digest = ruleDocumentDigest(document);
      if (row === undefined) {
        const enabled = entry.enabledByDefault ?? true;
        this.#upsert(document, { applicability: "project", now, sourceDigest: digest, origin: "rerail", enabled });
        report.inserted += 1;
        continue;
      }
      const read = this.#stored(row);
      let refresh: boolean;
      if (read.ok) {
        const existing = read.stored;
        refresh = existing.sourceDigest !== digest && existing.sourceDigest !== null
          && ruleDocumentDigest({ kind: existing.kind, rule: existing.rule } as RuleDocument) === existing.sourceDigest;
      } else {
        // An untouched bundled row the schema now rejects is replaced by its bundled document; a user's row is kept.
        refresh = row.sourceDigest !== null && row.kind === document.kind
          && storedDocumentDigest(row.kind, row.document.rule) === row.sourceDigest;
      }
      if (refresh) {
        this.#upsert(document, { applicability: "project", now, sourceDigest: digest, origin: "rerail" });
        report.refreshed += 1;
      } else {
        report.kept += 1;
      }
    }
    return report;
  }


  /** Change token covering both other connections and writes through this instance. */
  dataVersion(): string {
    const sqliteVersion = this.#database.all<{ data_version: number }>(sql`PRAGMA data_version`)[0]?.data_version ?? 0;
    return `${sqliteVersion}:${this.#localRevision}`;
  }

  ruleSignature(kind: RuleKind): string {
    return scopedRuleSignature(this.#database, this.#repository.repositoryId, kind);
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#database.$client.close();
  }

  #ensureRepository(now: number): void {
    this.#database.insert(ruleRepositories).values({
      repositoryId: this.#repository.repositoryId,
      lastKnownPath: this.#repository.workspaceRoot,
      createdAt: now,
      updatedAt: now,
    }).onConflictDoUpdate({
      target: ruleRepositories.repositoryId,
      set: { lastKnownPath: this.#repository.workspaceRoot, updatedAt: now },
    }).run();
  }

  #project(): { projectId: string; name: string } | undefined {
    return this.#database.select({ projectId: ruleProjects.projectId, name: ruleProjects.name })
      .from(ruleProjectRepositories)
      .innerJoin(ruleProjects, eq(ruleProjectRepositories.projectId, ruleProjects.projectId))
      .where(eq(ruleProjectRepositories.repositoryId, this.#repository.repositoryId)).get();
  }

  #owner(applicability: RuleApplicability): { kind: RuleApplicability; id: string } {
    if (applicability === "global") return { kind: "global", id: GLOBAL_OWNER };
    return { kind: "project", id: this.context().projectId };
  }

  #upsert(document: RuleDocument, options: ScopedRuleWriteOptions): ScopedStoredRule {
    // Every catalog write lands here; the engine compiles a session's rules all or nothing.
    if (document.kind === "conversation") assertEngineRegexes(document.rule);
    const stored = this.#database.transaction(() => {
      const owner = this.#owner(options.applicability ?? "project");
      this.#database.insert(ruleCatalogIdentities).values({ ruleId: document.rule.id, kind: document.kind })
        .onConflictDoNothing().run();
      const identity = this.#database.select().from(ruleCatalogIdentities)
        .where(eq(ruleCatalogIdentities.ruleId, document.rule.id)).get();
      if (identity?.kind !== document.kind) {
        throw new Error(`rule ${document.rule.id} exists with kind ${identity?.kind}; remove it before storing ${document.kind}`);
      }
      const conflicting = this.#database.select({ kind: catalogRules.kind }).from(catalogRules)
        .where(eq(catalogRules.ruleId, document.rule.id)).get();
      if (conflicting !== undefined && conflicting.kind !== document.kind) {
        throw new Error(`rule ${document.rule.id} exists with kind ${conflicting.kind}; remove it before storing ${document.kind}`);
      }
      // Metadata comes from the raw row so an unreadable row being replaced keeps its provenance.
      const existing = this.#ownedRow(owner.kind, owner.id, document.rule.id);
      const now = options.now ?? Date.now();
      let revision: number;
      if (document.kind === "file") revision = document.rule.revision;
      else if (options.revision !== undefined) revision = options.revision;
      else revision = existing === undefined ? 1 : existing.revision + 1;
      const payloadHash = ruleDocumentDigest(document);
      this.#database.insert(catalogRules).values({
        ownerKind: owner.kind,
        ownerId: owner.id,
        ruleId: document.rule.id,
        kind: document.kind,
        revision,
        enabled: options.enabled ?? existing?.enabled ?? true,
        document,
        payloadHash,
        sourceDigest: options.sourceDigest === undefined ? existing?.sourceDigest ?? null : options.sourceDigest,
        origin: options.origin ?? "user",
        createdAt: existing?.createdAt ?? options.createdAt ?? now,
        updatedAt: now,
      }).onConflictDoUpdate({
        target: [catalogRules.ownerKind, catalogRules.ownerId, catalogRules.ruleId],
        set: {
          kind: document.kind,
          revision,
          enabled: options.enabled ?? existing?.enabled ?? true,
          document,
          payloadHash,
          sourceDigest: options.sourceDigest === undefined ? existing?.sourceDigest ?? null : options.sourceDigest,
          origin: options.origin ?? existing?.origin ?? "user",
          updatedAt: now,
        },
      }).run();
      return this.#owned(owner.kind, owner.id, document.rule.id)!;
    }, { behavior: "immediate" });
    this.#localRevision += 1;
    return stored;
  }

  #ownedRow(kind: RuleApplicability, ownerId: string, ruleId: string): CatalogRuleRow | undefined {
    return this.#database.select().from(catalogRules).where(and(
      eq(catalogRules.ownerKind, kind), eq(catalogRules.ownerId, ownerId), eq(catalogRules.ruleId, ruleId),
    )).get();
  }

  #owned(kind: RuleApplicability, ownerId: string, ruleId: string): ScopedStoredRule | undefined {
    const row = this.#ownedRow(kind, ownerId, ruleId);
    return row === undefined ? undefined : this.#read(row);
  }

  #ownedRows(): CatalogRuleRow[] {
    const projectId = this.context().projectId;
    return this.#database.select().from(catalogRules)
      .where(inArray(catalogRules.ownerId, [GLOBAL_OWNER, projectId]))
      .orderBy(asc(catalogRules.ownerKind), asc(catalogRules.ruleId)).all();
  }

  /** Project rows shadow global rows before validation, so broken overrides cannot resurrect globals. */
  #effectiveOwnedRows(): CatalogRuleRow[] {
    const effective = new Map<string, CatalogRuleRow>();
    for (const row of this.#ownedRows()) {
      if (row.ownerKind === "project" || !effective.has(row.ruleId)) effective.set(row.ruleId, row);
    }
    return [...effective.values()];
  }

  #effectiveRows(): ScopedStoredRule[] {
    return this.#effectiveOwnedRows().flatMap(row => this.#read(row) ?? []).sort(compareRuleIds);
  }

  #read(row: CatalogRuleRow): ScopedStoredRule | undefined {
    const read = this.#stored(row);
    if (read.ok) return read.stored;
    reportQuarantinedRule(this.path, read.quarantined);
    return undefined;
  }

  /** A row that fails validation is quarantined, never thrown. */
  #stored(row: CatalogRuleRow): { ok: true; stored: ScopedStoredRule } | { ok: false; quarantined: QuarantinedRule } {
    const quarantine = (reason: string) => ({
      ok: false as const,
      quarantined: { ruleId: row.ruleId, kind: row.kind, owner: row.ownerKind === "global" ? GLOBAL_OWNER : `project:${row.ownerId}`, reason },
    });
    let document: RuleDocument;
    if (row.kind === "file") {
      const read = readStoredCodeRule(row.document.rule);
      if (!read.ok) return quarantine(read.reason);
      document = { kind: "file", rule: read.rule };
    } else {
      try {
        document = { kind: "conversation", rule: parseConversationRule(row.document.rule) };
      } catch (error) {
        return quarantine(error instanceof Error ? error.message : String(error));
      }
    }
    const base = {
      enabled: row.enabled,
      applicability: row.ownerKind,
      origin: row.ownerKind === "global" ? { kind: "global" as const } : { kind: "project" as const, projectId: row.ownerId },
      revision: row.revision,
      createdAt: row.createdAt,
      updatedAt: row.updatedAt,
      sourceDigest: row.sourceDigest,
      payloadHash: row.payloadHash,
      catalogOrigin: row.origin,
    };
    return { ok: true, stored: { ...base, ...document } };
  }

}
