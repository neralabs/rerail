/**
 * Rule configuration: which rules a repository runs where, and with which `on` / `mode` / `suppressions` / `config`.
 * Attachments make packs, By Rerail rules and library rules present in a folder; settings owned by a checkout,
 * its parent checkouts or the repository change them. See `repository-model.ts` for the precedence.
 */
import { createHash } from "node:crypto";
import { and, eq, inArray, or, sql } from "drizzle-orm";
import type { RerailDatabase } from "../storage/database.js";
import {
  folderContains,
  normalizeFolder,
  type RuleCatalog,
  type CheckoutContext,
  type CheckoutRuleSet,
  type Decider,
  type EffectiveRule,
  type LibraryRule,
  type RepositoryKey,
  type RepositoryStatus,
  type RuleAttachment,
  type RuleConfig,
  type RuleSetting,
  type RuleSuppressions,
  type RuleSourceKind,
  type UnavailableReference,
} from "./repository-model.js";
import { readLibraryRow } from "./rule-library.js";
import type { CodeRuleDslValue } from "../code-rule/code-rule.js";
import type { RuleMode } from "./rule-mode.js";
import type { RulePackRule } from "./rule-packs.js";
import type { RuleDocument } from "./rule-store.js";
import { libraryRules, repositories, ruleApprovals, ruleAttachments, ruleSettings, type RuleKind } from "../storage/schema.js";
import { clearSetupReview } from "./setup-review.js";
import { compareDeterministicStrings, ruleDocumentDigest, stableJsonText } from "../stable-json.js";
import { configureBundledCodeRule } from "./rule-config.js";
import { isWorktreePack, WORKTREE_PACK_ID } from "./worktree-pack.js";
import { approvalKey, packSubject, readApprovals, ruleSubject, type RuleApprovalSubject } from "./rule-approval.js";

type Queries = Pick<RerailDatabase, "insert" | "delete" | "select" | "update">;
type AttachmentRow = typeof ruleAttachments.$inferSelect;
type SettingRow = typeof ruleSettings.$inferSelect;
type LibraryRow = typeof libraryRules.$inferSelect;

function ownerColumns(owner: RuleSetting["owner"]): { ownerKind: "repository" | "checkout"; ownerId: string } {
  return owner.kind === "repository"
    ? { ownerKind: "repository", ownerId: owner.repositoryKey }
    : { ownerKind: "checkout", ownerId: owner.checkoutPath };
}

function insertAttachment(db: Queries, attachment: RuleAttachment, now: number): void {
  db.insert(ruleAttachments)
    .values({ ...attachment, folder: normalizeFolder(attachment.folder), createdAt: now })
    .onConflictDoNothing()
    .run();
}

function writeSetting(db: Queries, setting: RuleSetting, now: number): void {
  const { ownerKind, ownerId } = ownerColumns(setting.owner);
  const folder = normalizeFolder(setting.folder);
  const suppressions = setting.suppressions ?? null;
  const config = setting.config === undefined || setting.config === null || Object.keys(setting.config).length === 0 ? null : setting.config;
  if (setting.on === null && setting.mode === null && suppressions === null && config === null) {
    db.delete(ruleSettings).where(and(
      eq(ruleSettings.ownerKind, ownerKind),
      eq(ruleSettings.ownerId, ownerId),
      eq(ruleSettings.folder, folder),
      eq(ruleSettings.targetKind, setting.targetKind),
      eq(ruleSettings.targetId, setting.targetId),
    )).run();
    return;
  }
  const values = { on: setting.on, mode: setting.mode, suppressions, config };
  db.insert(ruleSettings)
    .values({ ownerKind, ownerId, folder, targetKind: setting.targetKind, targetId: setting.targetId, ...values, updatedAt: now })
    .onConflictDoUpdate({
      target: [ruleSettings.ownerKind, ruleSettings.ownerId, ruleSettings.folder, ruleSettings.targetKind, ruleSettings.targetId],
      set: { ...values, updatedAt: now },
    })
    .run();
}

/** Add a pack, By Rerail rule or library rule to a repository folder; adding it twice changes nothing. */
export function addAttachment(db: Queries, attachment: RuleAttachment, now: number = Date.now()): void {
  insertAttachment(db, attachment, now);
}

/** Remove one attachment; false when it was not there. */
export function removeAttachment(db: Queries, attachment: RuleAttachment): boolean {
  return db.delete(ruleAttachments).where(and(
    eq(ruleAttachments.repositoryKey, attachment.repositoryKey),
    eq(ruleAttachments.folder, normalizeFolder(attachment.folder)),
    eq(ruleAttachments.sourceKind, attachment.sourceKind),
    eq(ruleAttachments.sourceId, attachment.sourceId),
  )).returning({ id: ruleAttachments.sourceId }).get() !== undefined;
}

/** Store a setting, replacing the row's fields; with every field null (or omitted) the setting is removed. */
export function setRuleSetting(db: Queries, setting: RuleSetting, now: number = Date.now()): void {
  writeSetting(db, setting, now);
}

export interface SetUpChoice {
  readonly packs: readonly { readonly packId: string; readonly folder: string }[];
  readonly rules?: readonly { readonly sourceKind: RuleSourceKind; readonly sourceId: string; readonly folder: string }[];
  readonly settings?: readonly RuleSetting[];
}

/** Records the set-up choice in one transaction and marks the repository set up, even when nothing is added. */
export function setUpRepository(db: RerailDatabase, repositoryKey: RepositoryKey, choice: SetUpChoice, now: number = Date.now()): void {
  db.transaction(tx => recordSetUp(tx, repositoryKey, choice, now), { behavior: "immediate" });
}

/** {@link setUpRepository} inside a transaction the caller owns. */
export function recordSetUp(tx: Queries, repositoryKey: RepositoryKey, choice: SetUpChoice, now: number = Date.now()): void {
  const row = tx.select({ key: repositories.repositoryKey }).from(repositories).where(eq(repositories.repositoryKey, repositoryKey)).get();
  if (row === undefined) throw new Error(`Unknown repository: ${repositoryKey}`);
  for (const pack of choice.packs) insertAttachment(tx, { repositoryKey, folder: pack.folder, sourceKind: "pack", sourceId: pack.packId }, now);
  for (const rule of choice.rules ?? []) insertAttachment(tx, { repositoryKey, ...rule }, now);
  for (const setting of choice.settings ?? []) {
    if (setting.owner.kind !== "repository" || setting.owner.repositoryKey !== repositoryKey) {
      throw new Error(`Set-up settings must belong to repository ${repositoryKey}`);
    }
    writeSetting(tx, setting, now);
  }
  tx.update(repositories).set({ configuredAt: now, updatedAt: now }).where(eq(repositories.repositoryKey, repositoryKey)).run();
  // Set up now: an earlier set-up's offer of the review no longer stands (`setup-review.ts`).
  clearSetupReview(tx, repositoryKey);
}

/** Everything one load needs, read once. */
interface ConfigurationRows {
  readonly configuredAt: number | null;
  readonly attachments: readonly AttachmentRow[];
  readonly settings: readonly SettingRow[];
  readonly library: readonly LibraryRow[];
  /** Every approval recorded (`rule-approval.ts`), by key. */
  readonly approvals: ReadonlySet<string>;
}

/** A checkout's own rules (`worktree-pack.ts`): in its catalog alone, and present in it for the whole checkout. */
function worktreeAttachments(ruleCatalog: RuleCatalog, repositoryKey: RepositoryKey): AttachmentRow[] {
  return isWorktreePack(ruleCatalog.packs.get(WORKTREE_PACK_ID))
    ? [{ repositoryKey, folder: "", sourceKind: "pack", sourceId: WORKTREE_PACK_ID, createdAt: 0 }]
    : [];
}

/** Attached library rules, and library rules that shadow a present By Rerail rule, pack members included. */
function libraryIdsOf(ruleCatalog: RuleCatalog, attachments: readonly Pick<AttachmentRow, "sourceKind" | "sourceId">[]): string[] {
  return [...new Set(attachments.flatMap(row => row.sourceKind === "pack"
    ? (ruleCatalog.packs.get(row.sourceId)?.rules ?? []).map(rule => rule.id)
    : [row.sourceId]))];
}

function readRows(db: RerailDatabase, ruleCatalog: RuleCatalog, repositoryKey: RepositoryKey, ownerCheckoutPaths: readonly string[]): ConfigurationRows {
  const repository = db.select({ configuredAt: repositories.configuredAt }).from(repositories).where(eq(repositories.repositoryKey, repositoryKey)).get();
  const attachments = [...db.select().from(ruleAttachments).where(eq(ruleAttachments.repositoryKey, repositoryKey)).all(), ...worktreeAttachments(ruleCatalog, repositoryKey)];
  const repositoryOwned = and(eq(ruleSettings.ownerKind, "repository"), eq(ruleSettings.ownerId, repositoryKey));
  const settings = db.select().from(ruleSettings).where(ownerCheckoutPaths.length === 0
    ? repositoryOwned
    : or(repositoryOwned, and(eq(ruleSettings.ownerKind, "checkout"), inArray(ruleSettings.ownerId, [...ownerCheckoutPaths])))).all();
  const ids = libraryIdsOf(ruleCatalog, attachments);
  const library = ids.length === 0 ? [] : db.select().from(libraryRules).where(inArray(libraryRules.ruleId, ids)).all();
  return { configuredAt: repository?.configuredAt ?? null, attachments, settings, library, approvals: readApprovals(db) };
}

/** What {@link signatureOf} hashes: the fields of {@link ConfigurationRows} that {@link signatureStatement} reads. */
interface SignatureRows {
  readonly configuredAt: number | null;
  readonly attachments: readonly Pick<AttachmentRow, "folder" | "sourceKind" | "sourceId">[];
  readonly settings: readonly Omit<SettingRow, "updatedAt">[];
  readonly library: readonly Pick<LibraryRow, "ruleId" | "revision" | "payloadHash">[];
  readonly approvals: ReadonlySet<string>;
}

function signatureOf(catalogDigest: string, ownerCheckoutPaths: readonly string[], rows: SignatureRows): string {
  const lines = [
    `catalog\0${catalogDigest}`,
    `configured\0${String(rows.configuredAt)}`,
    `owners\0${ownerCheckoutPaths.join("\0")}`,
    ...rows.attachments.map(row => `attachment\0${row.folder}\0${row.sourceKind}\0${row.sourceId}`).sort(compareDeterministicStrings),
    ...rows.settings
      .map(row => [
        `setting\0${row.ownerKind}\0${row.ownerId}\0${row.folder}\0${row.targetKind}\0${row.targetId}`,
        `${String(row.on)}\0${String(row.mode)}\0${String(row.suppressions)}\0${stableJsonText(row.config) ?? "null"}`,
      ].join("\0"))
      .sort(compareDeterministicStrings),
    ...rows.library
      .map(row => `library\0${row.ruleId}\0${row.revision}\0${row.payloadHash}`)
      .sort(compareDeterministicStrings),
    ...[...rows.approvals].map(key => `approval\0${key}`).sort(compareDeterministicStrings),
  ];
  return `sha256:${createHash("sha256").update(lines.join("\n")).digest("hex")}`;
}

/**
 * The rows {@link ruleConfigurationSignature} hashes, in one row: the repository's set-up time and, as JSON arrays, its
 * attachments, the settings of the repository and of the given checkouts, every library rule and every approval. Library
 * rules are few, and which of them count depends on the rule catalog's packs, so they are picked in {@link signatureRowsOf}.
 */
function signatureStatement(db: RerailDatabase) {
  const key = sql.placeholder("key");
  const owners = sql.placeholder("owners");
  return db.select({
    configuredAt: sql<number | null>`(SELECT ${repositories.configuredAt} FROM ${repositories} WHERE ${repositories.repositoryKey} = ${key})`,
    attachments: sql<string>`(SELECT json_group_array(json_array(${ruleAttachments.folder}, ${ruleAttachments.sourceKind}, ${ruleAttachments.sourceId}))
      FROM ${ruleAttachments} WHERE ${ruleAttachments.repositoryKey} = ${key})`,
    settings: sql<string>`(SELECT json_group_array(json_array(${ruleSettings.ownerKind}, ${ruleSettings.ownerId}, ${ruleSettings.folder},
      ${ruleSettings.targetKind}, ${ruleSettings.targetId}, ${ruleSettings.on}, ${ruleSettings.mode}, ${ruleSettings.suppressions}, json(${ruleSettings.config})))
      FROM ${ruleSettings} WHERE (${ruleSettings.ownerKind} = 'repository' AND ${ruleSettings.ownerId} = ${key})
      OR (${ruleSettings.ownerKind} = 'checkout' AND ${ruleSettings.ownerId} IN (SELECT value FROM json_each(${owners}))))`,
    library: sql<string>`(SELECT json_group_array(json_array(${libraryRules.ruleId}, ${libraryRules.revision}, ${libraryRules.payloadHash})) FROM ${libraryRules})`,
    approvals: sql<string>`(SELECT json_group_array(json_array(${ruleApprovals.subjectKind}, ${ruleApprovals.subjectId}, ${ruleApprovals.digest})) FROM ${ruleApprovals})`,
  }).from(sql`(SELECT 1)`).prepare();
}

type SignatureRead = NonNullable<ReturnType<ReturnType<typeof signatureStatement>["get"]>>;

/** One JSON array of {@link signatureStatement}; SQL built it from the table's columns, so each tuple has their types. */
function tuples<Tuple extends readonly unknown[]>(json: string): Tuple[] {
  return JSON.parse(json) as Tuple[]; // Built by json_group_array over the table's own columns.
}

/**
 * {@link SignatureRows} from {@link signatureStatement}'s row, with the checkout's own rules and the library rules that
 * count picked as {@link readRows} picks them, so the signature changes whenever a load would read other rows.
 */
function signatureRowsOf(read: SignatureRead, ruleCatalog: RuleCatalog, repositoryKey: RepositoryKey): SignatureRows {
  const attachments = [
    ...tuples<[string, AttachmentRow["sourceKind"], string]>(read.attachments).map(([folder, sourceKind, sourceId]) => ({ folder, sourceKind, sourceId })),
    ...worktreeAttachments(ruleCatalog, repositoryKey),
  ];
  const ids = new Set(libraryIdsOf(ruleCatalog, attachments));
  type Setting = [SettingRow["ownerKind"], string, string, SettingRow["targetKind"], string, number | null, SettingRow["mode"], SettingRow["suppressions"], SettingRow["config"]];
  return {
    configuredAt: read.configuredAt,
    attachments,
    settings: tuples<Setting>(read.settings).map(([ownerKind, ownerId, folder, targetKind, targetId, on, mode, suppressions, config]) =>
      ({ ownerKind, ownerId, folder, targetKind, targetId, on: on === null ? null : on === 1, mode, suppressions, config })),
    library: tuples<[string, number, string]>(read.library).filter(([ruleId]) => ids.has(ruleId)).map(([ruleId, revision, payloadHash]) => ({ ruleId, revision, payloadHash })),
    approvals: new Set(tuples<[RuleApprovalSubject["kind"], string, string]>(read.approvals).map(([kind, id, digest]) => approvalKey({ kind, id, digest }))),
  };
}

/** A connection's {@link signatureStatement}, and the last signature of each checkout with the row it was hashed from. */
interface SignatureReader {
  readonly statement: ReturnType<typeof signatureStatement>;
  readonly last: Map<string, { readonly read: string; readonly signature: string }>;
}

const signatureReaders = new WeakMap<RerailDatabase, SignatureReader>();

/** Checkouts whose last signature a connection keeps: more than a runtime knows, and a bound on what it keeps. */
const KEPT_SIGNATURES = 1024;

/**
 * Changes whenever the rule catalog, the repository's set-up, attachments or settings, the settings of the given checkouts
 * (the checkout and its parents), a library rule that is attached or shadows a present By Rerail rule, or an approval
 * changes. Not the start state. Every lane a session opens and every refresh asks it, so it reads one statement prepared
 * once per connection, and hashes again only when that statement's row differs from the one it hashed last for the same
 * checkout: a change made by any process is read at the next call.
 */
export function ruleConfigurationSignature(
  db: RerailDatabase,
  ruleCatalog: RuleCatalog,
  repositoryKey: RepositoryKey,
  ownerCheckoutPaths: readonly string[],
): string {
  let reader = signatureReaders.get(db);
  if (reader === undefined) {
    reader = { statement: signatureStatement(db), last: new Map() };
    signatureReaders.set(db, reader);
  }
  const row = reader.statement.get({ key: repositoryKey, owners: JSON.stringify(ownerCheckoutPaths) })!;
  const read = JSON.stringify(row);
  const checkout = JSON.stringify([ruleCatalog.digest, repositoryKey, ownerCheckoutPaths]);
  const last = reader.last.get(checkout);
  if (last?.read === read) return last.signature;
  const signature = signatureOf(ruleCatalog.digest, ownerCheckoutPaths, signatureRowsOf(row, ruleCatalog, repositoryKey));
  if (reader.last.size >= KEPT_SIGNATURES) reader.last.clear();
  reader.last.set(checkout, { read, signature });
  return signature;
}

/** A rule present at a path, before its settings are applied. */
interface Presence {
  readonly document: RuleDocument;
  readonly source: EffectiveRule["source"];
  readonly folder: string;
  readonly packRule: RulePackRule | null;
  readonly revision: number;
  readonly payloadHash: string;
}

type Owner = { readonly kind: "checkout"; readonly id: string } | { readonly kind: "repository"; readonly id: string };

/** Deepest folder first; within one folder the rule's own setting before its pack's. */
function compareSettings(left: SettingRow, right: SettingRow): number {
  if (left.folder.length !== right.folder.length) return right.folder.length - left.folder.length;
  if (left.targetKind === right.targetKind) return 0;
  return left.targetKind === "rule" ? -1 : 1;
}

function bundledPresenceFields(document: RuleDocument): Pick<Presence, "revision" | "payloadHash"> {
  return { revision: document.kind === "code" ? document.rule.revision : 1, payloadHash: ruleDocumentDigest(document) };
}

/** Resolves effective rules at any path from rows read once. */
class ConfigurationResolver {
  readonly unavailable: readonly UnavailableReference[];
  readonly #ruleCatalog: RuleCatalog;
  readonly #attachments: readonly AttachmentRow[];
  readonly #owners: readonly Owner[];
  readonly #settingsByOwner = new Map<string, SettingRow[]>();
  readonly #library = new Map<string, LibraryRule>();
  /** Library ids that exist at all (readable or not); a By Rerail rule with one of these ids is shadowed. */
  readonly #libraryIds: ReadonlySet<string>;
  readonly #bundledFields = new Map<string, Pick<Presence, "revision" | "payloadHash">>();
  readonly #approvals: ReadonlySet<string>;

  constructor(ruleCatalog: RuleCatalog, rows: ConfigurationRows, owners: readonly Owner[]) {
    this.#ruleCatalog = ruleCatalog;
    this.#owners = owners;
    this.#approvals = rows.approvals;
    // Deepest attachment first, so a rule present through two folders reports the deepest one.
    this.#attachments = [...rows.attachments].sort((left, right) => right.folder.length - left.folder.length);
    for (const setting of rows.settings) {
      const key = `${setting.ownerKind}\0${setting.ownerId}`;
      this.#settingsByOwner.set(key, [...(this.#settingsByOwner.get(key) ?? []), setting]);
    }
    for (const list of this.#settingsByOwner.values()) list.sort(compareSettings);
    this.#libraryIds = new Set(rows.library.map(row => row.ruleId));
    for (const row of rows.library) {
      const read = readLibraryRow(row);
      if (read.ok) this.#library.set(row.ruleId, read.rule);
    }
    this.unavailable = rows.attachments.flatMap((row): UnavailableReference[] => {
      const reference = { sourceKind: row.sourceKind, sourceId: row.sourceId, folder: row.folder };
      if (row.sourceKind === "pack") {
        const pack = ruleCatalog.packs.get(row.sourceId);
        if (pack === undefined) return [{ ...reference, reason: "missing" }];
        // A pack rule a library rule shadows is reported by its own id, at the pack's folder.
        return pack.rules
          .filter(rule => this.#libraryIds.has(rule.id))
          .map(rule => ({ sourceKind: "rerail_rule", sourceId: rule.id, folder: row.folder, reason: "conflicts-with-library" }));
      }
      if (row.sourceKind === "library_rule") return this.#library.has(row.sourceId) ? [] : [{ ...reference, reason: "missing" }];
      if (this.#libraryIds.has(row.sourceId)) return [{ ...reference, reason: "conflicts-with-library" }];
      return ruleCatalog.documents.has(row.sourceId) ? [] : [{ ...reference, reason: "missing" }];
    });
  }

  /** Every rule present at `path` (relative to the repository root), with its effective `on` and `mode`. */
  at(path: string, kind: RuleKind): EffectiveRule[] {
    return [...this.#presences(path).values()]
      .filter(presence => presence.document.kind === kind)
      .map(presence => this.#effective(presence, path))
      .sort((left, right) => compareDeterministicStrings(left.document.rule.id, right.document.rule.id));
  }

  #bundled(id: string): { document: RuleDocument; fields: Pick<Presence, "revision" | "payloadHash"> } | undefined {
    const document = this.#ruleCatalog.documents.get(id);
    if (document === undefined || this.#libraryIds.has(id)) return undefined;
    let fields = this.#bundledFields.get(id);
    if (fields === undefined) {
      fields = bundledPresenceFields(document);
      this.#bundledFields.set(id, fields);
    }
    return { document, fields };
  }

  #presences(path: string): Map<string, Presence> {
    const present = new Map<string, Presence>();
    const candidates = this.#attachments.filter(row => folderContains(row.folder, path));
    for (const row of candidates.filter(candidate => candidate.sourceKind === "library_rule")) {
      const rule = this.#library.get(row.sourceId);
      if (rule === undefined || present.has(row.sourceId)) continue;
      present.set(row.sourceId, {
        document: rule.document,
        source: { kind: "library_rule", id: row.sourceId, packId: null },
        folder: row.folder,
        packRule: null,
        revision: rule.revision,
        payloadHash: rule.payloadHash,
      });
    }
    for (const row of candidates.filter(candidate => candidate.sourceKind === "pack")) {
      for (const packRule of this.#ruleCatalog.packs.get(row.sourceId)?.rules ?? []) {
        const bundled = this.#bundled(packRule.id);
        if (bundled === undefined || present.has(packRule.id)) continue;
        present.set(packRule.id, {
          document: bundled.document,
          source: { kind: "pack", id: row.sourceId, packId: row.sourceId },
          folder: row.folder,
          packRule,
          ...bundled.fields,
        });
      }
    }
    for (const row of candidates.filter(candidate => candidate.sourceKind === "rerail_rule")) {
      const bundled = this.#bundled(row.sourceId);
      if (bundled === undefined || present.has(row.sourceId)) continue;
      present.set(row.sourceId, {
        document: bundled.document,
        source: { kind: "rerail_rule", id: row.sourceId, packId: null },
        folder: row.folder,
        packRule: null,
        ...bundled.fields,
      });
    }
    return present;
  }

  #decide<T>(presence: Presence, path: string, field: (setting: SettingRow) => T | null): { value: T; decider: Decider } | undefined {
    const id = presence.document.rule.id;
    const packId = presence.source.packId;
    for (const owner of this.#owners) {
      for (const setting of this.#settingsByOwner.get(`${owner.kind}\0${owner.id}`) ?? []) {
        const targets = setting.targetKind === "rule" ? setting.targetId === id : packId !== null && setting.targetId === packId;
        const value = field(setting);
        if (!targets || value === null || !folderContains(setting.folder, path)) continue;
        return { value, decider: deciderOf(owner, setting) };
      }
    }
    return undefined;
  }

  /**
   * The option values settings give a code rule by Rerail, per option: the nearest owner's setting for the rule over
   * the whole repository (folder `""`) that sets the option. A library rule takes none.
   */
  #config(presence: Presence): { config: RuleConfig | null; deciders: Record<string, Decider> } {
    const config: Record<string, CodeRuleDslValue> = {};
    const deciders: Record<string, Decider> = {};
    if (presence.document.kind !== "code" || presence.source.kind === "library_rule") return { config: null, deciders };
    const id = presence.document.rule.id;
    for (const owner of this.#owners) {
      for (const setting of this.#settingsByOwner.get(`${owner.kind}\0${owner.id}`) ?? []) {
        if (setting.targetKind !== "rule" || setting.targetId !== id || setting.folder !== "" || setting.config === null) continue;
        for (const [name, value] of Object.entries(setting.config)) {
          if (Object.hasOwn(config, name)) continue;
          config[name] = value;
          deciders[name] = deciderOf(owner, setting);
        }
      }
    }
    return { config: Object.keys(config).length === 0 ? null : config, deciders };
  }

  /**
   * What the rule's content waits for a person to approve before it steers (`rule-approval.ts`): the rule itself when
   * it is created by you or a worktree's own, its pack when that is not Rerail's own from npm; undefined otherwise.
   */
  #approvalSubject(presence: Presence): RuleApprovalSubject | undefined {
    const id = presence.document.rule.id;
    if (presence.source.kind === "library_rule") return ruleSubject(id, presence.payloadHash);
    const packId = presence.source.packId ?? this.#ruleCatalog.packOf.get(id);
    const pack = packId === undefined ? undefined : this.#ruleCatalog.packs.get(packId);
    if (pack === undefined) return undefined;
    return isWorktreePack(pack) ? ruleSubject(id, presence.payloadHash) : packSubject(this.#ruleCatalog, pack);
  }

  #effective(presence: Presence, path: string): EffectiveRule {
    const { packRule } = presence;
    const packId = presence.source.packId;
    const start = packRule === null || packId === null ? undefined : { rule: packRule, decider: { by: "pack-start", packId } as Decider };
    const ruleDefault: Decider = { by: "rule-default" };
    const on = this.#decide<boolean>(presence, path, setting => setting.on)
      ?? (start === undefined ? { value: true, decider: ruleDefault } : { value: start.rule.on, decider: start.decider });
    const configuredMode = this.#decide<RuleMode>(presence, path, setting => setting.mode)
      ?? (start === undefined ? { value: presence.document.rule.mode, decider: ruleDefault } : { value: start.rule.mode, decider: start.decider });
    const subject = this.#approvalSubject(presence);
    const approval = subject === undefined ? undefined : { subject, approved: this.#approvals.has(approvalKey(subject)) };
    // Content no one approved is recorded, and never steers.
    const mode = approval?.approved === false && configuredMode.value !== "log"
      ? { value: "log" as RuleMode, decider: { by: "awaiting-approval" } as Decider }
      : configuredMode;
    const pack = packId === null ? undefined : this.#ruleCatalog.packs.get(packId);
    const suppressions = this.#decide<RuleSuppressions>(presence, path, setting => setting.suppressions)
      ?? (start === undefined || pack === undefined
        ? { value: "honour", decider: ruleDefault }
        : { value: pack.honourSuppressionComments ? "honour" : "ignore", decider: start.decider });
    const { config, deciders } = this.#config(presence);
    const configured = config === null ? undefined : configureBundledCodeRule(this.#ruleCatalog, presence.document.rule.id, config);
    const rule = configured?.ok === true ? configured.rule : presence.document.rule;
    const document = { kind: presence.document.kind, rule: { ...rule, mode: mode.value } } as RuleDocument;
    return {
      document,
      source: presence.source,
      attachmentFolder: presence.folder,
      on: on.value,
      mode: mode.value,
      suppressions: suppressions.value,
      config,
      configError: configured?.ok === false ? configured.reason : null,
      decidedBy: { on: on.decider, mode: mode.decider, suppressions: suppressions.decider, config: deciders },
      revision: presence.revision,
      payloadHash: presence.payloadHash,
      ...(approval === undefined ? {} : { approval }),
    };
  }
}

function deciderOf(owner: Owner, setting: SettingRow): Decider {
  return owner.kind === "checkout"
    ? { by: "checkout", checkoutPath: owner.id, folder: setting.folder, target: setting.targetKind }
    : { by: "repository", folder: setting.folder, target: setting.targetKind };
}

function ownersOf(repositoryKey: RepositoryKey, checkoutPaths: readonly string[]): Owner[] {
  return [...checkoutPaths.map((id): Owner => ({ kind: "checkout", id })), { kind: "repository", id: repositoryKey }];
}

/**
 * The effective configuration for one checkout. `rulesFor` and `conversationRules` list every present rule, off ones
 * included (`on: false`); both are empty unless the checkout is started and its repository set up. Every row is read
 * here, once; lookups never query. {@link ruleConfigurationSignature} tells whether the configuration changed.
 */
export function loadCheckoutRuleSet(
  db: RerailDatabase,
  ruleCatalog: RuleCatalog,
  checkout: CheckoutContext,
  status: RepositoryStatus,
  parentCheckoutPaths: readonly string[],
): CheckoutRuleSet {
  const ownerCheckoutPaths = [checkout.checkoutPath, ...parentCheckoutPaths];
  const rows = readRows(db, ruleCatalog, checkout.repositoryKey, ownerCheckoutPaths);
  const active = status.started && status.setUp;
  const resolver = new ConfigurationResolver(ruleCatalog, rows, ownersOf(checkout.repositoryKey, ownerCheckoutPaths));
  const codeRules = new Map<string, readonly EffectiveRule[]>();
  const conversationRules = new Map<string, readonly EffectiveRule[]>();
  const memoised = (cache: Map<string, readonly EffectiveRule[]>, path: string, kind: RuleKind): readonly EffectiveRule[] => {
    if (!active) return [];
    let rules = cache.get(path);
    if (rules === undefined) {
      rules = Object.freeze(resolver.at(path, kind));
      cache.set(path, rules);
    }
    return rules;
  };
  return {
    checkout,
    status,
    rulesFor: relativePath => memoised(codeRules, normalizeFolder(relativePath), "code"),
    conversationRules: startFolder => memoised(conversationRules, normalizeFolder(startFolder), "conversation"),
    unavailable: active ? resolver.unavailable : [],
  };
}

export interface RepositoryRulesView {
  /** Each present rule at its attachment folder, both kinds, with the source of `on` and `mode`. */
  readonly rules: readonly EffectiveRule[];
  readonly unavailable: readonly UnavailableReference[];
}

/**
 * The dashboard view of a repository's rules: every rule at the folder of the attachment that makes it present,
 * resolved for the given checkout (and its parents) or, without one, for the repository alone.
 */
export function listRepositoryRules(
  db: RerailDatabase,
  ruleCatalog: RuleCatalog,
  repositoryKey: RepositoryKey,
  options: { readonly checkoutPath?: string; readonly parentCheckoutPaths?: readonly string[] } = {},
): RepositoryRulesView {
  const checkoutPaths = options.checkoutPath === undefined ? [] : [options.checkoutPath, ...(options.parentCheckoutPaths ?? [])];
  const rows = readRows(db, ruleCatalog, repositoryKey, checkoutPaths);
  const resolver = new ConfigurationResolver(ruleCatalog, rows, ownersOf(repositoryKey, checkoutPaths));
  const folders = [...new Set(rows.attachments.map(row => row.folder))].sort(compareDeterministicStrings);
  const rules = folders.flatMap(folder => (["code", "conversation"] as const)
    .flatMap(kind => resolver.at(folder, kind))
    .filter(rule => rule.attachmentFolder === folder));
  return { rules, unavailable: resolver.unavailable };
}
