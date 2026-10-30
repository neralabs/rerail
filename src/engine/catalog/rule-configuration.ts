/**
 * Rule configuration: which rules a repository runs where, and with which `on` / `mode` / `suppressions` / `config`.
 * Attachments make packs, By Rerail rules and library rules present in a folder; settings owned by a checkout,
 * its parent checkouts or the repository change them. See `repository-model.ts` for the precedence.
 */
import { createHash } from "node:crypto";
import { and, eq, inArray, or } from "drizzle-orm";
import type { RerailDatabase } from "../storage/database.js";
import {
  folderContains,
  normalizeFolder,
  type RuleCatalog,
  type CheckoutContext,
  type CheckoutRuleSet,
  type Decider,
  type EffectiveRule,
  type RepositoryKey,
  type RepositoryStatus,
  type RuleApprovalStatus,
  type RuleAttachment,
  type RuleConfig,
  type RuleSetting,
  type RuleSuppressions,
  type RuleSourceKind,
  type UnavailableReference,
} from "./repository-model.js";
import { approvalKey, approvedContent, catalogRevision, readApprovalRow, type RuleApproval } from "./rule-approval.js";
import { readLibraryRow } from "./rule-library.js";
import type { CodeRuleDslValue } from "../code-rule/code-rule.js";
import type { RuleMode } from "./rule-mode.js";
import type { RulePackRule } from "./rule-packs.js";
import type { RuleDocument } from "./rule-store.js";
import { libraryRules, repositories, ruleApprovals, ruleAttachments, ruleSettings, type RuleKind } from "../storage/schema.js";
import { clearSetupReview } from "./setup-review.js";
import { compareDeterministicStrings, ruleContentDigest, ruleDocumentDigest, stableJsonText } from "../stable-json.js";
import { configureBundledCodeRule } from "./rule-config.js";

type Queries = Pick<RerailDatabase, "insert" | "delete" | "select" | "update">;
type AttachmentRow = typeof ruleAttachments.$inferSelect;
type SettingRow = typeof ruleSettings.$inferSelect;
type LibraryRow = typeof libraryRules.$inferSelect;
type ApprovalRow = typeof ruleApprovals.$inferSelect;

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
  readonly approvals: readonly ApprovalRow[];
}

function readRows(db: RerailDatabase, ruleCatalog: RuleCatalog, repositoryKey: RepositoryKey, ownerCheckoutPaths: readonly string[]): ConfigurationRows {
  const repository = db.select({ configuredAt: repositories.configuredAt }).from(repositories).where(eq(repositories.repositoryKey, repositoryKey)).get();
  const attachments = db.select().from(ruleAttachments).where(eq(ruleAttachments.repositoryKey, repositoryKey)).all();
  const repositoryOwned = and(eq(ruleSettings.ownerKind, "repository"), eq(ruleSettings.ownerId, repositoryKey));
  const settings = db.select().from(ruleSettings).where(ownerCheckoutPaths.length === 0
    ? repositoryOwned
    : or(repositoryOwned, and(eq(ruleSettings.ownerKind, "checkout"), inArray(ruleSettings.ownerId, [...ownerCheckoutPaths])))).all();
  // Attached library rules, and library rules that shadow a present By Rerail rule, pack members included.
  const ids = [...new Set(attachments.flatMap(row => row.sourceKind === "pack"
    ? (ruleCatalog.packs.get(row.sourceId)?.rules ?? []).map(rule => rule.id)
    : [row.sourceId]))];
  const library = ids.length === 0 ? [] : db.select().from(libraryRules).where(inArray(libraryRules.ruleId, ids)).all();
  const approvals = ids.length === 0 ? [] : db.select().from(ruleApprovals).where(inArray(ruleApprovals.ruleId, ids)).all();
  return { configuredAt: repository?.configuredAt ?? null, attachments, settings, library, approvals };
}

function signatureOf(catalogDigest: string, ownerCheckoutPaths: readonly string[], rows: ConfigurationRows): string {
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
    // The stored content too: a row whose content no longer matches its hash stops approving it.
    ...rows.approvals
      .map(row => `approval\0${row.owner}\0${row.ruleId}\0${row.revision}\0${row.contentHash}\0${row.approvedAt}\0${stableJsonText(row.document) ?? ""}`)
      .sort(compareDeterministicStrings),
  ];
  return `sha256:${createHash("sha256").update(lines.join("\n")).digest("hex")}`;
}

/**
 * Changes whenever the rule catalog, the repository's set-up, attachments or settings, the settings of the given checkouts
 * (the checkout and its parents), or a library rule that is attached or shadows a present By Rerail rule changes.
 * Not the start state.
 */
export function ruleConfigurationSignature(
  db: RerailDatabase,
  ruleCatalog: RuleCatalog,
  repositoryKey: RepositoryKey,
  ownerCheckoutPaths: readonly string[],
): string {
  return signatureOf(ruleCatalog.digest, ownerCheckoutPaths, readRows(db, ruleCatalog, repositoryKey, ownerCheckoutPaths));
}

/** What a present rule runs, before its settings are applied. */
interface Content {
  readonly document: RuleDocument;
  readonly revision: number;
  readonly payloadHash: string;
  readonly approval: RuleApprovalStatus;
}

/** A rule present at a path, before its settings are applied. */
interface Presence extends Content {
  readonly source: EffectiveRule["source"];
  readonly folder: string;
  readonly packRule: RulePackRule | null;
}

type Owner = { readonly kind: "checkout"; readonly id: string } | { readonly kind: "repository"; readonly id: string };

/** Deepest folder first; within one folder the rule's own setting before its pack's. */
function compareSettings(left: SettingRow, right: SettingRow): number {
  if (left.folder.length !== right.folder.length) return right.folder.length - left.folder.length;
  if (left.targetKind === right.targetKind) return 0;
  return left.targetKind === "rule" ? -1 : 1;
}

/** Resolves effective rules at any path from rows read once. */
class ConfigurationResolver {
  readonly unavailable: readonly UnavailableReference[];
  readonly #ruleCatalog: RuleCatalog;
  readonly #attachments: readonly AttachmentRow[];
  readonly #owners: readonly Owner[];
  readonly #settingsByOwner = new Map<string, SettingRow[]>();
  readonly #approvals = new Map<string, RuleApproval>();
  /** Library rules with approved content, as they run. */
  readonly #library = new Map<string, Content>();
  /** Readable library rules with no approved content: they run nowhere and shadow nothing. */
  readonly #awaitingLibrary = new Set<string>();
  /** Library ids that shadow a By Rerail rule of the same id: every row (readable or not) but one awaiting approval. */
  readonly #libraryIds: ReadonlySet<string>;
  /** Catalog rules by id, as they run; null for one that awaits approval. */
  readonly #bundledContent = new Map<string, Content | null>();

  constructor(ruleCatalog: RuleCatalog, rows: ConfigurationRows, owners: readonly Owner[]) {
    this.#ruleCatalog = ruleCatalog;
    this.#owners = owners;
    // Deepest attachment first, so a rule present through two folders reports the deepest one.
    this.#attachments = [...rows.attachments].sort((left, right) => right.folder.length - left.folder.length);
    for (const setting of rows.settings) {
      const key = `${setting.ownerKind}\0${setting.ownerId}`;
      this.#settingsByOwner.set(key, [...(this.#settingsByOwner.get(key) ?? []), setting]);
    }
    for (const list of this.#settingsByOwner.values()) list.sort(compareSettings);
    for (const row of rows.approvals) {
      const approval = readApprovalRow(row);
      if (approval !== undefined) this.#approvals.set(approvalKey(row.owner, row.ruleId), approval);
    }
    for (const row of rows.library) {
      const read = readLibraryRow(row);
      if (!read.ok) continue;
      const running = approvedContent(read.rule, this.#approvals.get(approvalKey("library", row.ruleId)));
      if (running === undefined) this.#awaitingLibrary.add(row.ruleId);
      else {
        const payloadHash = running.approval.state === "approved" ? read.rule.payloadHash : ruleDocumentDigest(running.document);
        this.#library.set(row.ruleId, { ...running, payloadHash });
      }
    }
    this.#libraryIds = new Set(rows.library.map(row => row.ruleId).filter(id => !this.#awaitingLibrary.has(id)));
    this.unavailable = rows.attachments.flatMap((row): UnavailableReference[] => {
      const reference = { sourceKind: row.sourceKind, sourceId: row.sourceId, folder: row.folder };
      if (row.sourceKind === "pack") {
        const pack = ruleCatalog.packs.get(row.sourceId);
        if (pack === undefined) return [{ ...reference, reason: "missing" }];
        // A pack rule a library rule shadows, or one that awaits approval, is reported by its own id, at the pack's folder.
        return pack.rules.flatMap((rule): UnavailableReference[] => {
          const own = { sourceKind: "rerail_rule" as const, sourceId: rule.id, folder: row.folder };
          if (this.#libraryIds.has(rule.id)) return [{ ...own, reason: "conflicts-with-library" }];
          return this.#bundled(rule.id) === null ? [{ ...own, reason: "awaiting-approval" }] : [];
        });
      }
      if (row.sourceKind === "library_rule") {
        if (this.#library.has(row.sourceId)) return [];
        return [{ ...reference, reason: this.#awaitingLibrary.has(row.sourceId) ? "awaiting-approval" : "missing" }];
      }
      if (this.#libraryIds.has(row.sourceId)) return [{ ...reference, reason: "conflicts-with-library" }];
      const bundled = this.#bundled(row.sourceId);
      if (bundled === undefined) return [{ ...reference, reason: "missing" }];
      return bundled === null ? [{ ...reference, reason: "awaiting-approval" }] : [];
    });
  }

  /** Every rule present at `path` (relative to the repository root), with its effective `on` and `mode`. */
  at(path: string, kind: RuleKind): EffectiveRule[] {
    return [...this.#presences(path).values()]
      .filter(presence => presence.document.kind === kind)
      .map(presence => this.#effective(presence, path))
      .sort((left, right) => compareDeterministicStrings(left.document.rule.id, right.document.rule.id));
  }

  /**
   * What catalog rule `id` runs: as shipped for a rule Nera shipped, else its approved content (`rule-approval.ts`);
   * null when it has none, undefined when the catalog does not hold it or a library rule shadows it.
   */
  #bundled(id: string): Content | null | undefined {
    const document = this.#ruleCatalog.documents.get(id);
    if (document === undefined || this.#libraryIds.has(id)) return undefined;
    let content = this.#bundledContent.get(id);
    if (content === undefined) {
      const revision = catalogRevision(document);
      if (this.#ruleCatalog.shipped.has(id)) {
        content = { document, revision, payloadHash: ruleDocumentDigest(document), approval: { state: "shipped", approvedAt: null } };
      } else {
        const running = approvedContent(
          { document, revision, contentHash: ruleContentDigest(document.kind, document.rule) },
          this.#approvals.get(approvalKey("pack", id)),
        );
        content = running === undefined ? null : { ...running, payloadHash: ruleDocumentDigest(running.document) };
      }
      this.#bundledContent.set(id, content);
    }
    return content;
  }

  #presences(path: string): Map<string, Presence> {
    const present = new Map<string, Presence>();
    const candidates = this.#attachments.filter(row => folderContains(row.folder, path));
    for (const row of candidates.filter(candidate => candidate.sourceKind === "library_rule")) {
      const content = this.#library.get(row.sourceId);
      if (content === undefined || present.has(row.sourceId)) continue;
      present.set(row.sourceId, { ...content, source: { kind: "library_rule", id: row.sourceId, packId: null }, folder: row.folder, packRule: null });
    }
    for (const row of candidates.filter(candidate => candidate.sourceKind === "pack")) {
      for (const packRule of this.#ruleCatalog.packs.get(row.sourceId)?.rules ?? []) {
        const content = this.#bundled(packRule.id);
        if (content === undefined || content === null || present.has(packRule.id)) continue;
        present.set(packRule.id, { ...content, source: { kind: "pack", id: row.sourceId, packId: row.sourceId }, folder: row.folder, packRule });
      }
    }
    for (const row of candidates.filter(candidate => candidate.sourceKind === "rerail_rule")) {
      const content = this.#bundled(row.sourceId);
      if (content === undefined || content === null || present.has(row.sourceId)) continue;
      present.set(row.sourceId, { ...content, source: { kind: "rerail_rule", id: row.sourceId, packId: null }, folder: row.folder, packRule: null });
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
   * the whole repository (folder `""`) that sets the option. A library rule takes none, nor does a pack rule running
   * the content approved before while newer content waits: options apply to the pack's current document.
   */
  #config(presence: Presence): { config: RuleConfig | null; deciders: Record<string, Decider> } {
    const config: Record<string, CodeRuleDslValue> = {};
    const deciders: Record<string, Decider> = {};
    if (presence.document.kind !== "code" || presence.source.kind === "library_rule" || presence.approval.state === "pending") {
      return { config: null, deciders };
    }
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

  #effective(presence: Presence, path: string): EffectiveRule {
    const { packRule } = presence;
    const packId = presence.source.packId;
    const start = packRule === null || packId === null ? undefined : { rule: packRule, decider: { by: "pack-start", packId } as Decider };
    const ruleDefault: Decider = { by: "rule-default" };
    const on = this.#decide<boolean>(presence, path, setting => setting.on)
      ?? (start === undefined ? { value: true, decider: ruleDefault } : { value: start.rule.on, decider: start.decider });
    const mode = this.#decide<RuleMode>(presence, path, setting => setting.mode)
      ?? (start === undefined ? { value: presence.document.rule.mode, decider: ruleDefault } : { value: start.rule.mode, decider: start.decider });
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
      approval: presence.approval,
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
  /**
   * How many of the repository's rules have content waiting for approval: those that run nowhere yet (unavailable,
   * `awaiting-approval`) and those that run their approved content while an edit waits.
   */
  readonly awaitingApproval: number;
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
  const awaiting = new Set([
    ...resolver.unavailable.filter(reference => reference.reason === "awaiting-approval").map(reference => reference.sourceId),
    ...rules.filter(rule => rule.approval.state === "pending").map(rule => rule.document.rule.id),
  ]);
  return { rules, unavailable: resolver.unavailable, awaitingApproval: awaiting.size };
}
