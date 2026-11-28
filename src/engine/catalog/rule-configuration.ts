/**
 * Rule configuration: which rules a repository runs where, and with which `on` / `mode` / `suppressions` / `config`.
 * Attachments make packs, By Rerail rules and library rules present in a folder; settings owned by a checkout,
 * its parent checkouts or the repository change them. See `repository-model.ts` for the precedence.
 */
import { createHash } from "node:crypto";
import { and, eq, getTableColumns, inArray, or, sql, type SQL } from "drizzle-orm";
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
import { readLibraryRow, storedLibraryRows, type StoredLibraryRow } from "./rule-library.js";
import { stderrRowLog, storedObject, type StoredRowLog } from "./rule-quarantine.js";
import type { CodeRuleDslValue } from "../code-rule/code-rule.js";
import type { RuleMode } from "./rule-mode.js";
import type { RulePackRule } from "./rule-packs.js";
import { ruleKindOf, type RuleDocument } from "./rule-store.js";
import { libraryRules, repositories, ruleApprovals, ruleAttachments, ruleSettings, type RuleKind } from "../storage/schema.js";
import { clearSetupReview } from "./setup-review.js";
import { nameOnce } from "../storage/stored-list.js";
import { compareDeterministicStrings, ruleDocumentDigest, stableJsonText } from "../stable-json.js";
import { configureBundledCodeRule } from "./rule-config.js";
import { isWorktreePack, WORKTREE_PACK_ID } from "./worktree-pack.js";
import { approvalKey, packSubject, readApprovals, ruleSubject, type RuleApprovalSubject } from "./rule-approval.js";

type Queries = Pick<RerailDatabase, "insert" | "delete" | "select" | "update">;
type AttachmentRow = typeof ruleAttachments.$inferSelect;
type SettingRow = typeof ruleSettings.$inferSelect;
type LibraryRow = typeof libraryRules.$inferSelect;

/**
 * A `rule_settings` row with `config_json` as the text the database holds, which `readSettings` parses, so a setting
 * whose text a damaged file or a hand edit left costs its own rule's options, never every rule of the repository.
 */
const STORED_SETTING_COLUMNS = { ...getTableColumns(ruleSettings), config: sql<string | null>`${ruleSettings.config}` };
type StoredSettingRow = Omit<SettingRow, "config"> & { readonly config: string | null };
/** A setting as read: its options, or none where their stored text holds no JSON object, and then why (`unread`). */
export type ReadSetting = SettingRow & { readonly unread: string | null };

/**
 * The settings `where` selects, in `order`, each with its options read (`readConfig`). A row whose options do not read
 * is named once in `named`'s log.
 */
export function readSettings(
  db: Pick<RerailDatabase, "select">,
  query: { readonly where?: SQL | undefined; readonly order?: readonly SQL[]; readonly named?: StoredRowLog },
): ReadSetting[] {
  const rows: StoredSettingRow[] = db.select(STORED_SETTING_COLUMNS).from(ruleSettings).where(query.where).orderBy(...(query.order ?? [])).all();
  return rows.map(row => {
    const read = readConfig(row.config);
    if (read.unread !== null && query.named !== undefined) sayUnreadConfig(query.named, row, read.unread);
    return { ...row, ...read };
  });
}

/** Stored options text as a setting reads it: the object it holds, or none and why it holds none. */
function readConfig(text: string | null): Pick<ReadSetting, "config" | "unread"> {
  const stored = text === null ? null : storedObject(text);
  if (typeof stored === "string") return { config: null, unread: stored };
  // The type the ORM's JSON column gave these values: Rerail writes only option values there, and no reader checked them.
  return { config: stored as RuleConfig | null, unread: null };
}

/** Names, once in `named`'s log, the setting whose stored options are no JSON object, and why. */
function sayUnreadConfig({ file, log }: StoredRowLog, row: StoredSettingRow, why: string): void {
  // The column name the schema gives, so the line names the place a person mends.
  const column = ruleSettings.config.name;
  const setting = `the ${row.targetKind} setting of ${row.targetId} by ${row.ownerKind} ${row.ownerId} at folder "${row.folder}"`;
  nameOnce(log, { file, place: `rule setting ${row.ownerKind} ${row.ownerId} ${row.folder} ${row.targetKind} ${row.targetId}` },
    `rerail: ${file}: ${setting} reads with no options, and a rule by Rerail it gives options runs nowhere until they are set again: `
    + `the ${column} of its row is no JSON object (${why}), which only a damaged file or a hand edit leaves.\n`);
}

/** Why a rule whose setting holds options that do not read runs nowhere (`configError`). */
function unreadOptionsError(why: string): string {
  return `the options a setting gives it in the database are no JSON object (${why}), which only a damaged file or a hand edit leaves; `
    + "set them again to run it";
}

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

/** Option values as a row stores them: none (`null`) for no options. */
const storedConfig = (config: RuleConfig | null | undefined): RuleConfig | null =>
  (config === undefined || config === null || Object.keys(config).length === 0 ? null : config);

/**
 * Writes `setting`'s row, or deletes it when every field is null. The row takes `setting.origin`, null when omitted:
 * a write that does not say the lint config gave it (the dashboard, the API, MCP) makes the row a person's.
 */
function writeSetting(db: Queries, setting: RuleSetting, now: number): void {
  const { ownerKind, ownerId } = ownerColumns(setting.owner);
  const key = { ownerKind, ownerId, folder: normalizeFolder(setting.folder), targetKind: setting.targetKind, targetId: setting.targetId };
  const values = { on: setting.on, mode: setting.mode, suppressions: setting.suppressions ?? null, config: storedConfig(setting.config), origin: setting.origin ?? null };
  if (values.on === null && values.mode === null && values.suppressions === null && values.config === null) {
    db.delete(ruleSettings).where(and(
      eq(ruleSettings.ownerKind, key.ownerKind),
      eq(ruleSettings.ownerId, key.ownerId),
      eq(ruleSettings.folder, key.folder),
      eq(ruleSettings.targetKind, key.targetKind),
      eq(ruleSettings.targetId, key.targetId),
    )).run();
    return;
  }
  db.insert(ruleSettings)
    .values({ ...key, ...values, updatedAt: now })
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
  readonly settings: readonly ReadSetting[];
  readonly library: readonly LibraryEntry[];
  /** Every approval recorded (`rule-approval.ts`), by key. */
  readonly approvals: ReadonlySet<string>;
}

/** A library rule row as the configuration reads it: what the signature hashes, and the rule, none where it is quarantined. */
type LibraryEntry = Pick<LibraryRow, "ruleId" | "revision" | "payloadHash"> & { readonly rule: LibraryRule | undefined };

function libraryEntry(row: StoredLibraryRow, named: StoredRowLog): LibraryEntry {
  const read = readLibraryRow(row, named);
  return { ruleId: row.ruleId, revision: row.revision, payloadHash: row.payloadHash, rule: read.ok ? read.rule : undefined };
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
  const named = stderrRowLog(db);
  const settings = readSettings(db, {
    where: ownerCheckoutPaths.length === 0
      ? repositoryOwned
      : or(repositoryOwned, and(eq(ruleSettings.ownerKind, "checkout"), inArray(ruleSettings.ownerId, [...ownerCheckoutPaths]))),
    named,
  });
  const ids = libraryIdsOf(ruleCatalog, attachments);
  const library = ids.length === 0 ? [] : storedLibraryRows(db, inArray(libraryRules.ruleId, ids)).map(row => libraryEntry(row, named));
  return { configuredAt: repository?.configuredAt ?? null, attachments, settings, library, approvals: readApprovals(db) };
}

/** What {@link signatureOf} hashes: {@link ConfigurationRows}, or the fields of them that one statement reads. */
interface SignatureRows {
  readonly configuredAt: number | null;
  readonly attachments: readonly Pick<AttachmentRow, "folder" | "sourceKind" | "sourceId">[];
  // A setting's origin says who made it and decides no rule, so the signature leaves it out, as it leaves updatedAt.
  readonly settings: readonly Omit<ReadSetting, "updatedAt" | "origin">[];
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
        `${String(row.on)}\0${String(row.mode)}\0${String(row.suppressions)}\0${optionsText(row)}`,
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
 * A setting's options as the signature hashes them: as before for options that read, so a configuration that reads keeps
 * its signature, and why they do not read for those that do not.
 */
function optionsText(row: Pick<ReadSetting, "config" | "unread">): string {
  return row.unread === null ? stableJsonText(row.config) ?? "null" : `unread\0${row.unread}`;
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
      ${ruleSettings.targetKind}, ${ruleSettings.targetId}, ${ruleSettings.on}, ${ruleSettings.mode}, ${ruleSettings.suppressions}, ${ruleSettings.config}))
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

/** {@link SignatureRows} from {@link signatureStatement}'s row: the same rows {@link readRows} gives the signature. */
function signatureRowsOf(read: SignatureRead, ruleCatalog: RuleCatalog, repositoryKey: RepositoryKey): SignatureRows {
  const attachments = [
    ...tuples<[string, AttachmentRow["sourceKind"], string]>(read.attachments).map(([folder, sourceKind, sourceId]) => ({ folder, sourceKind, sourceId })),
    ...worktreeAttachments(ruleCatalog, repositoryKey),
  ];
  const ids = new Set(libraryIdsOf(ruleCatalog, attachments));
  type Setting = [SettingRow["ownerKind"], string, string, SettingRow["targetKind"], string, number | null, SettingRow["mode"], SettingRow["suppressions"], string | null];
  return {
    configuredAt: read.configuredAt,
    attachments,
    settings: tuples<Setting>(read.settings).map(([ownerKind, ownerId, folder, targetKind, targetId, on, mode, suppressions, config]) =>
      ({ ownerKind, ownerId, folder, targetKind, targetId, on: on === null ? null : on === 1, mode, suppressions, ...readConfig(config) })),
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

/**
 * Reads, in one statement prepared once, every repository's set-up time, attachments and settings, every checkout's
 * settings, every library rule and every approval, as one text: it differs from the last one read whenever any process
 * changed what any checkout's {@link ruleConfigurationSignature} reads from the database. The tables hold a few rows per
 * rule set up, never one per checkout recorded.
 */
export function ruleConfigurationVersionReader(db: RerailDatabase): () => string {
  const statement = db.select({
    repositories: sql<string>`(SELECT json_group_array(json_array(${repositories.repositoryKey}, ${repositories.configuredAt})) FROM ${repositories})`,
    attachments: sql<string>`(SELECT json_group_array(json_array(${ruleAttachments.repositoryKey}, ${ruleAttachments.folder},
      ${ruleAttachments.sourceKind}, ${ruleAttachments.sourceId})) FROM ${ruleAttachments})`,
    settings: sql<string>`(SELECT json_group_array(json_array(${ruleSettings.ownerKind}, ${ruleSettings.ownerId}, ${ruleSettings.folder},
      ${ruleSettings.targetKind}, ${ruleSettings.targetId}, ${ruleSettings.on}, ${ruleSettings.mode}, ${ruleSettings.suppressions}, ${ruleSettings.config}))
      FROM ${ruleSettings})`,
    library: sql<string>`(SELECT json_group_array(json_array(${libraryRules.ruleId}, ${libraryRules.revision}, ${libraryRules.payloadHash})) FROM ${libraryRules})`,
    approvals: sql<string>`(SELECT json_group_array(json_array(${ruleApprovals.subjectKind}, ${ruleApprovals.subjectId}, ${ruleApprovals.digest})) FROM ${ruleApprovals})`,
  }).from(sql`(SELECT 1)`).prepare();
  return () => JSON.stringify(statement.get());
}

/**
 * {@link ruleConfigurationVersionReader}'s text, and each repository's name and on/off, which that leaves out: it
 * differs whenever any process changed what a repository's rules for the repository alone ({@link listRepositoryRules}
 * without a checkout) or its listing read. A session's records leave it as it is, unlike the database's own version.
 */
export function repositoryRulesVersionReader(db: RerailDatabase): () => string {
  const rules = ruleConfigurationVersionReader(db);
  const listed = db.select({
    repositories: sql<string>`(SELECT json_group_array(json_array(${repositories.repositoryKey}, ${repositories.displayName},
      ${repositories.started})) FROM ${repositories})`,
  }).from(sql`(SELECT 1)`).prepare();
  return () => `${rules()}|${listed.get()?.repositories ?? ""}`;
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
  return { revision: document.kind === "conversation" ? 1 : document.rule.revision, payloadHash: ruleDocumentDigest(document) };
}

/**
 * The document each rule runs as at each mode, by the rule: nothing changes a rule once read, so every checkout whose
 * settings run one rule at one mode, in every runtime, gets one document, and what is worked out from it once (its
 * stable text and digest, `ruleSourceText`) serves every checkout's catalog. A copy per checkout made each catalog
 * build serialize every rule again: a session's first take of 190 worktrees held the daemon's thread for seconds.
 */
const runningDocuments = new WeakMap<object, Map<RuleMode, RuleDocument>>();

/** `rule` of a document of `kind` at `mode`; a rule belongs to one document, so its kind never varies. */
function runningDocument(kind: RuleDocument["kind"], rule: RuleDocument["rule"], mode: RuleMode): RuleDocument {
  let byMode = runningDocuments.get(rule);
  if (byMode === undefined) {
    byMode = new Map();
    runningDocuments.set(rule, byMode);
  }
  let document = byMode.get(mode);
  if (document === undefined) {
    document = { kind, rule: { ...rule, mode } } as RuleDocument;
    byMode.set(mode, document);
  }
  return document;
}

/** Resolves effective rules at any path from rows read once. */
class ConfigurationResolver {
  readonly unavailable: readonly UnavailableReference[];
  readonly #ruleCatalog: RuleCatalog;
  readonly #attachments: readonly AttachmentRow[];
  readonly #owners: readonly Owner[];
  readonly #settingsByOwner: ReadonlyMap<string, readonly ReadSetting[]>;
  /** Why the options a setting gives a rule over the whole repository do not read, by the rule's id (`unreadRuleOptions`). */
  readonly #unreadOptions: ReadonlyMap<string, string>;
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
    this.#settingsByOwner = settingsByOwner(rows.settings);
    this.#unreadOptions = unreadRuleOptions(rows.settings);
    this.#libraryIds = new Set(rows.library.map(row => row.ruleId));
    for (const row of rows.library) if (row.rule !== undefined) this.#library.set(row.ruleId, row.rule);
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
      .filter(presence => ruleKindOf(presence.document) === kind)
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
   * the whole repository (folder `""`) that sets the option. A library rule takes none. A rule written in `.rerail`
   * takes none either, and values given it are reported as a configuration error (`configureBundledCodeRule`).
   */
  #config(presence: Presence): { config: RuleConfig | null; deciders: Record<string, Decider> } {
    if (presence.document.kind === "conversation" || presence.source.kind === "library_rule") return { config: null, deciders: {} };
    return this.#ruleConfig(presence.document.rule.id);
  }

  /**
   * Why options a setting gives `presence`'s rule do not read, where options apply to it (`#config`); undefined when they
   * read. Which options such a setting held is lost, so the rule runs nowhere rather than with some of them.
   */
  #unreadOptionsOf(presence: Presence): string | undefined {
    if (presence.document.kind === "conversation" || presence.source.kind === "library_rule") return undefined;
    return this.#unreadOptions.get(presence.document.rule.id);
  }

  /** The option values the settings of each owner, nearest first, give rule `id` over the whole repository. */
  #ruleConfig(id: string): { config: RuleConfig | null; deciders: Record<string, Decider> } {
    const config: Record<string, CodeRuleDslValue> = {};
    const deciders: Record<string, Decider> = {};
    const configures = (setting: ReadSetting): setting is ReadSetting & { config: RuleConfig } =>
      setting.targetKind === "rule" && setting.targetId === id && setting.folder === "" && setting.config !== null;
    for (const owner of this.#owners) {
      for (const setting of (this.#settingsByOwner.get(`${owner.kind}\0${owner.id}`) ?? []).filter(configures)) {
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
    const start = packStart(presence);
    const on = this.#decide<boolean>(presence, path, setting => setting.on)
      ?? (start === undefined ? { value: true, decider: { by: "rule-default" } as Decider } : { value: start.rule.on, decider: start.decider });
    const approval = this.#approval(presence);
    const mode = this.#mode(presence, path, start, approval);
    const suppressions = this.#suppressions(presence, path, start);
    const { rule, config, deciders, configError } = this.#configured(presence);
    return {
      document: runningDocument(presence.document.kind, rule, mode.value),
      source: presence.source,
      attachmentFolder: presence.folder,
      on: on.value,
      mode: mode.value,
      suppressions: suppressions.value,
      config,
      configError,
      // A rule created by you under a pack rule's id shadows it: its document is its own, not the pack's.
      notRun: presence.source.kind === "library_rule" ? null : this.#ruleCatalog.notRun.get(presence.document.rule.id) ?? null,
      decidedBy: { on: on.decider, mode: mode.decider, suppressions: suppressions.decider, config: deciders },
      revision: presence.revision,
      payloadHash: presence.payloadHash,
      ...(approval === undefined ? {} : { approval }),
    };
  }

  /** Whether a person approved the rule's content, where it waits for one (`#approvalSubject`). */
  #approval(presence: Presence): EffectiveRule["approval"] {
    const subject = this.#approvalSubject(presence);
    return subject === undefined ? undefined : { subject, approved: this.#approvals.has(approvalKey(subject)) };
  }

  /** The rule's mode at `path`: a setting's, else its pack's start, else the rule's own; Log while its content awaits approval. */
  #mode(presence: Presence, path: string, start: PackStart | undefined, approval: EffectiveRule["approval"]): Decided<RuleMode> {
    const configured: Decided<RuleMode> = this.#decide<RuleMode>(presence, path, setting => setting.mode)
      ?? (start === undefined ? { value: presence.document.rule.mode, decider: { by: "rule-default" } } : { value: start.rule.mode, decider: start.decider });
    // Content no one approved is recorded, and never steers.
    return approval?.approved === false && configured.value !== "log" ? { value: "log", decider: { by: "awaiting-approval" } } : configured;
  }

  /** Whether the rule honours suppression comments at `path`: a setting's answer, else its pack's, else it honours them. */
  #suppressions(presence: Presence, path: string, start: PackStart | undefined): Decided<RuleSuppressions> {
    const packId = presence.source.packId;
    const pack = packId === null ? undefined : this.#ruleCatalog.packs.get(packId);
    return this.#decide<RuleSuppressions>(presence, path, setting => setting.suppressions)
      ?? (start === undefined || pack === undefined
        ? { value: "honour", decider: { by: "rule-default" } }
        : { value: pack.honourSuppressionComments ? "honour" : "ignore", decider: start.decider });
  }

  /** The rule as its settings' options configure it, those options and who set each, and why they do not apply when they do not. */
  #configured(presence: Presence): { rule: RuleDocument["rule"]; config: RuleConfig | null; deciders: Record<string, Decider>; configError: string | null } {
    const unread = this.#unreadOptionsOf(presence);
    if (unread !== undefined) return { rule: presence.document.rule, config: null, deciders: {}, configError: unreadOptionsError(unread) };
    const { config, deciders } = this.#config(presence);
    const configured = config === null ? undefined : configureBundledCodeRule(this.#ruleCatalog, presence.document.rule.id, config);
    if (configured === undefined) return { rule: presence.document.rule, config, deciders, configError: null };
    return configured.ok ? { rule: configured.rule, config, deciders, configError: null } : { rule: presence.document.rule, config, deciders, configError: configured.reason };
  }
}

/** `settings` by owner (`<kind>\0<id>`), each owner's in the order `compareSettings` gives. */
function settingsByOwner(settings: readonly ReadSetting[]): Map<string, ReadSetting[]> {
  const byOwner = new Map<string, ReadSetting[]>();
  for (const setting of settings) {
    const key = `${setting.ownerKind}\0${setting.ownerId}`;
    byOwner.set(key, [...(byOwner.get(key) ?? []), setting]);
  }
  for (const list of byOwner.values()) list.sort(compareSettings);
  return byOwner;
}

/** Why options do not read, by the id of the rule a setting gives them over the whole repository, where `#ruleConfig` reads them. */
function unreadRuleOptions(settings: readonly ReadSetting[]): Map<string, string> {
  const unread = new Map<string, string>();
  for (const setting of settings) {
    if (setting.unread !== null && setting.targetKind === "rule" && setting.folder === "") unread.set(setting.targetId, setting.unread);
  }
  return unread;
}

/** A value of a rule's configuration and who decided it. */
interface Decided<T> {
  readonly value: T;
  readonly decider: Decider;
}

/** The on and mode a pack starts one of its rules at, and the pack as their decider. */
interface PackStart {
  readonly rule: RulePackRule;
  readonly decider: Decider;
}

function packStart(presence: Presence): PackStart | undefined {
  const { packRule } = presence;
  const packId = presence.source.packId;
  return packRule === null || packId === null ? undefined : { rule: packRule, decider: { by: "pack-start", packId } };
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
 * here, once; lookups never query. The signature covers the configuration and the start state.
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
  const signature = `${signatureOf(ruleCatalog.digest, ownerCheckoutPaths, rows)}:${status.started ? "started" : "stopped"}:${status.setUp ? "set-up" : "not-set-up"}`;
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
    signature,
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
