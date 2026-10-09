/**
 * Operations on repository-owned rule configuration: repositories and checkouts, effective rules,
 * settings, attachments, start and stop, the library, By Rerail rules and pack recommendations. Every operation
 * names its target in the query or body (`repository`, `checkout`, `folder`); an MCP caller's checkout is the
 * default. GETs never record a checkout. Coding agents may change a repository's attachments and settings only
 * while it is not set up, to add a rule they created in the same call, or to add the copy of a By Rerail rule they
 * edited; people may change anything.
 */
import { existsSync, realpathSync } from "node:fs";
import { isAbsolute, relative, sep } from "node:path";
import { and, eq } from "drizzle-orm";
import { z, ZodError } from "zod";
import type { RerailDatabase } from "../database.js";
import { CodeRuleCompileError, EngineUnavailableError } from "../code-rule-compile.js";
import {
  normalizeFolder,
  type ApiCaller,
  type CheckoutContext,
  type CheckoutRecord,
  type EffectiveRule,
  type LibraryRule,
  type RepositoryStartState,
  type RepositoryStatus,
  type RuleSetting,
  type RuleSettingTargetKind,
} from "../repository-model.js";
import { repositoryIdentityLocation } from "../repository-identity.js";
import { linkAlias, listRepositories, parentCheckouts, type RepositoryListing } from "../repository-registry.js";
import { checkoutStatus, setCheckoutStarted, setRepositoryStarted } from "../repository-start.js";
import { addAttachment, listRepositoryRules, recordSetUp, removeAttachment, setRuleSetting } from "../rule-configuration.js";
import {
  copyRerailRuleToLibrary,
  getLibraryRule,
  listLibraryRules,
  quarantinedLibraryRules,
  removeLibraryRule,
  validateLibraryDocument,
  writeLibraryRule,
} from "../rule-library.js";
import type { RuleMode } from "../rule-mode.js";
import { cachedRecommendRulePacks } from "../rule-packs.js";
import { checkouts, libraryRules, repositories, ruleAttachments, ruleSettings, RULE_KINDS } from "../schema.js";
import { ApiError, defineOperation, type Operation, type RerailApiStores } from "./operation.js";
import {
  attachmentRecordSchema,
  attachmentsSetBodySchema,
  inactiveOf,
  libraryRuleRecordSchema,
  libraryUpsertBodySchema,
  libraryUpsertResultSchema,
  linkBodySchema,
  quarantinedRuleSchema,
  repositoryContextSchema,
  repositoryRulesSchema,
  repositorySummarySchema,
  rerailRuleSchema,
  ruleSettingRecordSchema,
  settingsSetBodySchema,
  startBodySchema,
  startResultSchema,
  targetShape,
} from "./repository-contracts.js";
import { rulePackRecommendationsSchema } from "./rule-pack-contracts.js";
import { rulesDashboardUrl } from "./session-contracts.js";

interface TargetInput {
  readonly repository?: string | undefined;
  readonly checkout?: string | undefined;
  readonly folder?: string | undefined;
}

interface Target {
  readonly repository: RepositoryListing;
  readonly checkout: CheckoutContext | undefined;
  /** The folder the target names: `folder`, else the folder `checkout` points into, else the agent's folder. */
  readonly folder: string;
}

function folderOf(folder: string): string {
  try {
    return normalizeFolder(folder);
  } catch (error) {
    throw new ApiError(400, "invalid-folder", (error as Error).message);
  }
}

/** The repository `key` names, by its own key or an alias. */
function findRepository(db: RerailDatabase, key: string): RepositoryListing | undefined {
  return listRepositories(db).find(repository => repository.repositoryKey === key || repository.aliases.includes(key));
}

function requireRepository(db: RerailDatabase, key: string): RepositoryListing {
  const repository = findRepository(db, key);
  if (repository === undefined) throw new ApiError(404, "repository-not-found", `repository ${key} is not known to Rerail`);
  return repository;
}

/** `path` relative to `root` when it lies inside it (or is it); undefined outside. A folder named `..cache` is inside. */
export function within(root: string, path: string): string | undefined {
  const inside = relative(root, path);
  return inside === ".." || inside.startsWith(`..${sep}`) || isAbsolute(inside) ? undefined : inside;
}

/** One answer for a path that doesn't exist and one Rerail hasn't seen, so the API is no existence oracle. */
function checkoutNotFound(path: string): ApiError {
  return new ApiError(404, "checkout-not-found", `Rerail has not seen a checkout at ${path}. Start Rerail there first: rerail start`);
}

/**
 * The checkout `path` is in and the folder inside it, never recording one: the API only acts on checkouts the CLI,
 * hooks or a trusted runtime recorded. Writes need the checkout itself recorded, not just its clone.
 */
function checkoutAt(stores: RerailApiStores, path: string, write: boolean): { checkout: CheckoutContext; folder: string } {
  let physical: string;
  try {
    physical = realpathSync(path);
  } catch {
    throw checkoutNotFound(path);
  }
  const checkout = stores.checkouts()(physical);
  const recorded = checkout !== undefined && (!write || stores.database().select({ path: checkouts.checkoutPath }).from(checkouts)
    .where(eq(checkouts.checkoutPath, checkout.checkoutPath)).get() !== undefined);
  if (checkout === undefined || !recorded) throw checkoutNotFound(path);
  const inside = within(checkout.checkoutPath, physical);
  return { checkout, folder: inside === undefined ? "" : folderOf(inside.split(sep).join("/")) };
}

/**
 * The checkout and repository an operation acts on: the `checkout` (with `repository`, if given, matching it), else
 * the `repository`, else the calling agent's checkout. `write` targets default to the whole repository.
 */
function resolveTarget(stores: RerailApiStores, input: TargetInput, caller: ApiCaller, write: boolean): Target {
  const db = stores.database();
  const located = input.checkout !== undefined
    ? checkoutAt(stores, input.checkout, write)
    : input.repository === undefined && caller.kind === "agent" && caller.checkout !== undefined
      ? { ...checkoutAt(stores, caller.checkout, write), folder: caller.folder }
      : undefined;
  if (located === undefined && input.repository === undefined) {
    throw new ApiError(400, "target-required", "name a repository or a checkout");
  }
  const repository = requireRepository(db, located?.checkout.repositoryKey ?? input.repository!);
  if (located !== undefined && input.repository !== undefined && findRepository(db, input.repository)?.repositoryKey !== repository.repositoryKey) {
    throw new ApiError(400, "target-mismatch", `checkout ${located.checkout.checkoutPath} belongs to ${repository.repositoryKey}, not ${input.repository}`);
  }
  const folder = input.folder !== undefined ? folderOf(input.folder) : write ? "" : located?.folder ?? "";
  return { repository, checkout: located?.checkout, folder };
}

/** A coding agent changes a checkout's own settings or start state only in its own checkout or one nested in it. */
function guardAgentCheckout(caller: ApiCaller, checkout: CheckoutContext): void {
  if (caller.kind === "person") return;
  if (caller.checkout === undefined || within(caller.checkout, checkout.checkoutPath) === undefined) {
    throw new ApiError(403, "agent-not-permitted", "a coding agent changes checkout settings and start state only in its own checkout and the checkouts inside it");
  }
}

/** Refuse an agent's change to a set-up repository's attachments or settings unless `allowed` says why it may. */
function guardAgent(stores: RerailApiStores, caller: ApiCaller, target: Target, allowed = false): void {
  if (caller.kind === "person" || target.repository.configuredAt === null || allowed) return;
  const url = rulesDashboardUrl(stores.dashboardOrigin(), {
    repositoryKey: target.repository.repositoryKey,
    checkoutPath: target.checkout?.checkoutPath ?? null,
  });
  throw new ApiError(403, "agent-not-permitted",
    `${target.repository.displayName} is set up, so a coding agent can't change its rules for the whole repository. `
    + `Change them for this worktree instead, or ask the person to change them in the dashboard: ${url}`);
}

function stateOf(status: RepositoryStatus): RepositoryStartState {
  if (!status.started) return "stopped";
  return status.setUp ? "started" : "not-set-up";
}

function contextOfRecord(record: CheckoutRecord): CheckoutContext {
  return { checkoutPath: record.checkoutPath, repositoryKey: record.repositoryKey, cloneId: record.cloneId, git: true, main: true };
}

function effectiveStatus(db: RerailDatabase, checkout: CheckoutContext): RepositoryStatus {
  return checkoutStatus(db, checkout);
}

export function repositorySummary(repository: RepositoryListing): z.infer<typeof repositorySummarySchema> {
  return {
    repositoryKey: repository.repositoryKey,
    name: repository.displayName,
    aliases: [...repository.aliases],
    setUp: repository.configuredAt !== null,
    configuredAt: repository.configuredAt,
    started: repository.started,
    state: stateOf({ started: repository.started, setUp: repository.configuredAt !== null, startedBy: "repository" }),
    checkoutCount: repository.checkouts.length,
  };
}

/**
 * The session `repository_id`s of a repository's history: its key, its aliases, and the clone ids of its checkouts
 * (rows recorded before sessions carried the repository key). An unknown key matches only itself.
 */
export function historyRepositoryIds(stores: RerailApiStores, key: string): string[] {
  const repository = findRepository(stores.database(), key);
  if (repository === undefined) return [key];
  return [...new Set([repository.repositoryKey, ...repository.aliases, ...repository.checkouts.map(checkout => checkout.cloneId)])];
}

function effectiveRuleRecord(rule: EffectiveRule): z.infer<typeof repositoryRulesSchema>["rules"][number] {
  const shared = {
    source: { ...rule.source },
    folder: rule.attachmentFolder,
    on: rule.on,
    mode: rule.mode,
    onSource: rule.decidedBy.on,
    modeSource: rule.decidedBy.mode,
    revision: rule.revision,
    payloadHash: rule.payloadHash,
  };
  return rule.document.kind === "file"
    ? { kind: "file", rule: rule.document.rule, ...shared, ...inactiveOf("file", rule.document.rule) }
    : { kind: "conversation", rule: rule.document.rule, ...shared };
}

function libraryRecord(rule: LibraryRule): z.infer<typeof libraryRuleRecordSchema> {
  const shared = {
    revision: rule.revision,
    payloadHash: rule.payloadHash,
    copiedFromRuleId: rule.copiedFromRuleId,
    createdAt: rule.createdAt,
    updatedAt: rule.updatedAt,
  };
  return rule.document.kind === "file"
    ? { kind: "file", rule: rule.document.rule, ...shared, ...inactiveOf("file", rule.document.rule) }
    : { kind: "conversation", rule: rule.document.rule, ...shared };
}

interface SettingChange {
  readonly folder: string;
  readonly targetKind: RuleSettingTargetKind;
  readonly targetId: string;
  readonly on?: boolean | null | undefined;
  readonly mode?: RuleMode | null | undefined;
}

/**
 * One change per (folder, target): a later entry's fields override an earlier one's, and a field it omits keeps the
 * earlier value, so `{ on: false }` then `{ mode: "log" }` asks for both.
 */
function foldedChanges(changes: readonly SettingChange[]): SettingChange[] {
  const folded = new Map<string, SettingChange>();
  for (const change of changes) {
    const key = `${change.folder}\0${change.targetKind}\0${change.targetId}`;
    const earlier = folded.get(key);
    folded.set(key, {
      folder: change.folder,
      targetKind: change.targetKind,
      targetId: change.targetId,
      on: change.on === undefined ? earlier?.on : change.on,
      mode: change.mode === undefined ? earlier?.mode : change.mode,
    });
  }
  return [...folded.values()];
}

/** Whether the repository is not set up yet, read inside the caller's transaction. */
function notSetUp(db: Pick<RerailDatabase, "select">, repositoryKey: string): boolean {
  return db.select({ configuredAt: repositories.configuredAt }).from(repositories)
    .where(eq(repositories.repositoryKey, repositoryKey)).get()?.configuredAt === null;
}

/** The setting `change` asks for: an omitted field keeps the stored value. */
function mergedSetting(db: Pick<RerailDatabase, "select">, owner: RuleSetting["owner"], change: SettingChange): RuleSetting {
  const stored = db.select({ on: ruleSettings.on, mode: ruleSettings.mode }).from(ruleSettings).where(and(
    eq(ruleSettings.ownerKind, owner.kind),
    eq(ruleSettings.ownerId, owner.kind === "repository" ? owner.repositoryKey : owner.checkoutPath),
    eq(ruleSettings.folder, change.folder),
    eq(ruleSettings.targetKind, change.targetKind),
    eq(ruleSettings.targetId, change.targetId),
  )).get();
  return {
    owner,
    folder: change.folder,
    targetKind: change.targetKind,
    targetId: change.targetId,
    on: change.on === undefined ? stored?.on ?? null : change.on,
    mode: change.mode === undefined ? stored?.mode ?? null : change.mode,
  };
}

function settingRecord(setting: RuleSetting): z.infer<typeof ruleSettingRecordSchema> {
  return {
    owner: setting.owner.kind,
    ownerId: setting.owner.kind === "repository" ? setting.owner.repositoryKey : setting.owner.checkoutPath,
    folder: setting.folder,
    targetKind: setting.targetKind,
    targetId: setting.targetId,
    on: setting.on,
    mode: setting.mode,
  };
}

function attachmentsOf(db: RerailDatabase, repositoryKey: string): z.infer<typeof attachmentRecordSchema>[] {
  return db.select({ folder: ruleAttachments.folder, sourceKind: ruleAttachments.sourceKind, sourceId: ruleAttachments.sourceId })
    .from(ruleAttachments).where(eq(ruleAttachments.repositoryKey, repositoryKey))
    .orderBy(ruleAttachments.folder, ruleAttachments.sourceKind, ruleAttachments.sourceId).all();
}

/** A checkout of the repository that is on disk to scan: the target's, else the main checkout, else the most recently seen. */
function checkoutOnDisk(target: Target): string {
  if (target.checkout !== undefined && existsSync(target.checkout.checkoutPath)) return target.checkout.checkoutPath;
  const present = target.repository.checkouts.filter(checkout => existsSync(checkout.checkoutPath));
  const main = present.find(checkout => repositoryIdentityLocation(checkout.checkoutPath).main);
  const recent = [...present].sort((left, right) => right.lastSeenAt - left.lastSeenAt)[0];
  const chosen = main ?? recent;
  if (chosen === undefined) throw new ApiError(409, "no-checkout-on-disk", `no checkout of ${target.repository.repositoryKey} is on this machine's disk`);
  return chosen.checkoutPath;
}

/** Library and engine errors as API errors. */
function libraryWrite<T>(write: () => T): T {
  try {
    return write();
  } catch (error) {
    if (error instanceof ApiError) throw error;
    if (error instanceof CodeRuleCompileError || error instanceof ZodError) throw new ApiError(400, "invalid-rule", error.message);
    if (error instanceof EngineUnavailableError) throw new ApiError(503, "kernel-unavailable", error.message);
    throw new ApiError(409, "rule-conflict", (error as Error).message);
  }
}

function ruleSourceExists(stores: RerailApiStores, kind: "pack" | "rerail_rule" | "library_rule", id: string): boolean {
  const bundle = stores.bundle();
  if (kind === "pack") return bundle.packs.has(id);
  if (kind === "rerail_rule") return bundle.documents.has(id);
  return getLibraryRule(stores.database(), id) !== undefined;
}

const libraryRuleId = z.object({ id: z.string().min(1) });
const kindFilter = z.object({ kind: z.enum(RULE_KINDS).optional() }).strict();

export const repositoryOperations: readonly Operation[] = [
  defineOperation({
    id: "repositories_list",
    method: "GET",
    path: "/api/repositories",
    summary: "List every repository Rerail knows on this machine: key, name, aliases, whether it is set up and started, and how many checkouts it has.",
    output: z.object({ repositories: z.array(repositorySummarySchema) }),
    handler: stores => ({ repositories: listRepositories(stores.database()).map(repositorySummary) }),
  }),
  defineOperation({
    id: "repository_context_get",
    method: "GET",
    path: "/api/repository",
    summary: "Show one repository: key, aliases, set up and started, its clones and their checkouts (own and effective state, containing checkout), and the checkout and folder the target or calling agent is in. Without a target an MCP call uses the calling agent's checkout.",
    query: z.object({ ...targetShape }).strict(),
    output: repositoryContextSchema,
    handler: (stores, input, caller) => {
      const target = resolveTarget(stores, input, caller, false);
      const db = stores.database();
      const clones = new Map<string, z.infer<typeof repositoryContextSchema>["clones"][number]["checkouts"]>();
      for (const record of target.repository.checkouts) {
        const status = effectiveStatus(db, contextOfRecord(record));
        clones.set(record.cloneId, [...(clones.get(record.cloneId) ?? []), {
          checkoutPath: record.checkoutPath,
          started: record.started,
          state: stateOf(status),
          startedBy: status.startedBy,
          parentCheckoutPath: parentCheckouts(db, record)[0]?.checkoutPath ?? null,
          onDisk: existsSync(record.checkoutPath),
        }]);
      }
      return {
        repository: repositorySummary(target.repository),
        clones: [...clones].map(([cloneId, checkouts]) => ({ cloneId, checkouts })),
        checkout: target.checkout === undefined ? null : {
          checkoutPath: target.checkout.checkoutPath,
          folder: target.folder,
          state: stateOf(effectiveStatus(db, target.checkout)),
        },
      };
    },
  }),
  defineOperation({
    id: "repository_rules_get",
    method: "GET",
    path: "/api/repository/rules",
    summary: "List a repository's rules, each at the folder of the attachment that adds it, with its on/off and mode and what decided each (a checkout's or the repository's setting, the pack's starting value, or the rule's default). With a checkout, that checkout's own settings and its containing checkouts' apply. References that can't be resolved are listed as unavailable.",
    query: z.object({ ...targetShape }).strict(),
    output: repositoryRulesSchema,
    handler: (stores, input, caller) => {
      const target = resolveTarget(stores, input, caller, false);
      const db = stores.database();
      const view = listRepositoryRules(db, stores.bundle(), target.repository.repositoryKey, target.checkout === undefined ? {} : {
        checkoutPath: target.checkout.checkoutPath,
        parentCheckoutPaths: parentCheckouts(db, target.checkout).map(parent => parent.checkoutPath),
      });
      return {
        repositoryKey: target.repository.repositoryKey,
        checkoutPath: target.checkout?.checkoutPath ?? null,
        setUp: target.repository.configuredAt !== null,
        rules: view.rules.map(effectiveRuleRecord),
        unavailable: view.unavailable.map(reference => ({ ...reference })),
      };
    },
  }),
  defineOperation({
    id: "repository_settings_set",
    method: "PUT",
    path: "/api/repository/settings",
    summary: "Turn rules or whole packs on or off, or change their mode, for the repository or for one checkout (this worktree), optionally in one folder. null clears a value so the next owner decides; an omitted value is kept. A coding agent may change checkout settings at any time, and the repository's only before it is set up.",
    body: settingsSetBodySchema,
    output: z.object({ settings: z.array(ruleSettingRecordSchema) }),
    handler: (stores, input, caller) => {
      const target = resolveTarget(stores, input, caller, true);
      const db = stores.database();
      let owner: RuleSetting["owner"];
      if (input.owner === "checkout") {
        if (target.checkout === undefined) throw new ApiError(400, "target-required", "a checkout setting needs a checkout");
        guardAgentCheckout(caller, target.checkout);
        owner = { kind: "checkout", checkoutPath: target.checkout.checkoutPath };
      } else {
        guardAgent(stores, caller, target);
        owner = { kind: "repository", repositoryKey: target.repository.repositoryKey };
      }
      const settings = input.targets.map(change => mergedSetting(db, owner, {
        folder: target.folder, targetKind: change.kind, targetId: change.id, on: input.on, mode: input.mode,
      }));
      for (const setting of settings) setRuleSetting(db, setting);
      return { settings: settings.map(settingRecord) };
    },
  }),
  defineOperation({
    id: "repository_attachments_set",
    method: "PUT",
    path: "/api/repository/attachments",
    summary: "Add or remove rule packs, By Rerail rules and your library rules in a repository, per folder (whole repository by default), with optional repository settings. The first call that adds anything, or passes configure: true, sets the repository up; it does not start Rerail. A coding agent may do this only before the repository is set up.",
    body: attachmentsSetBodySchema,
    output: z.object({ repository: repositorySummarySchema, attachments: z.array(attachmentRecordSchema) }),
    handler: (stores, input, caller) => {
      const target = resolveTarget(stores, input, caller, true);
      guardAgent(stores, caller, target);
      const db = stores.database();
      const repositoryKey = target.repository.repositoryKey;
      const add = (input.add ?? []).map(entry => ({ ...entry, folder: folderOf(entry.folder ?? "") }));
      const remove = (input.remove ?? []).map(entry => ({ ...entry, folder: folderOf(entry.folder ?? "") }));
      const missing = add.find(entry => !ruleSourceExists(stores, entry.sourceKind, entry.sourceId));
      if (missing !== undefined) throw new ApiError(404, "rule-source-not-found", `${missing.sourceKind} ${missing.sourceId} does not exist`);
      const owner = { kind: "repository" as const, repositoryKey };
      const changes = foldedChanges((input.settings ?? []).map(change => ({ ...change, folder: folderOf(change.folder ?? "") })));
      // The attachments and the settings that go with them are one change: nobody sees a pack before its settings.
      db.transaction(tx => {
        const settings = changes.map(change => mergedSetting(tx, owner, change));
        if (notSetUp(tx, repositoryKey) && (add.length > 0 || input.configure === true)) {
          recordSetUp(tx, repositoryKey, {
            packs: add.filter(entry => entry.sourceKind === "pack").map(entry => ({ packId: entry.sourceId, folder: entry.folder })),
            rules: add.filter(entry => entry.sourceKind !== "pack"),
            settings,
            // `/api` writes are the person's in the dashboard, where set-up's review sets the repository up.
            inDashboard: caller.kind === "person",
          });
        } else {
          for (const entry of add) addAttachment(tx, { repositoryKey, ...entry });
          for (const setting of settings) setRuleSetting(tx, setting);
        }
        for (const entry of remove) removeAttachment(tx, { repositoryKey, ...entry });
      }, { behavior: "immediate" });
      return { repository: repositorySummary(requireRepository(db, repositoryKey)), attachments: attachmentsOf(db, repositoryKey) };
    },
  }),
  defineOperation({
    id: "repository_start",
    method: "POST",
    path: "/api/repository/start",
    summary: "Start Rerail in a checkout and the checkouts inside it (scope checkout), or in every checkout of the repository, clearing their own values (scope repository). A repository that is not set up is started but runs no rules until it is.",
    body: startBodySchema,
    output: startResultSchema,
    handler: (stores, input, caller) => startOrStop(stores, input, caller, true),
  }),
  defineOperation({
    id: "repository_stop",
    method: "POST",
    path: "/api/repository/stop",
    summary: "Stop Rerail in a checkout and the checkouts inside it (scope checkout), or in every checkout of the repository, clearing their own values (scope repository). Rules and history are kept.",
    body: startBodySchema,
    output: startResultSchema,
    handler: (stores, input, caller) => startOrStop(stores, input, caller, false),
  }),
  defineOperation({
    id: "repository_link",
    method: "POST",
    path: "/api/repository/aliases",
    summary: "Make another remote address (a rename, an ssh host alias, a mirror) resolve to this repository. People only.",
    body: linkBodySchema,
    output: z.object({ repositoryKey: z.string(), alias: z.string() }),
    handler: (stores, input, caller) => {
      if (caller.kind === "agent") throw new ApiError(403, "agent-not-permitted", "only a person can link repository addresses, in the dashboard or the CLI");
      const target = resolveTarget(stores, input, caller, true);
      try {
        return { repositoryKey: target.repository.repositoryKey, alias: linkAlias(stores.database(), target.repository.repositoryKey, input.address) };
      } catch (error) {
        throw new ApiError(409, "link-conflict", (error as Error).message);
      }
    },
  }),
  defineOperation({
    id: "rule_packs_recommend",
    method: "GET",
    path: "/api/rule-packs/recommendations",
    summary: "Recommend rule packs for a repository by matching a checkout's files and package.json dependencies against each pack, each with the folder to add it in (the whole repository for repository packs). Local; changes nothing. A checkout is rescanned at most once a minute.",
    query: z.object({ ...targetShape }).strict(),
    output: rulePackRecommendationsSchema,
    handler: (stores, input, caller) => {
      const target = resolveTarget(stores, input, caller, false);
      const checkoutPath = checkoutOnDisk(target);
      return {
        repositoryKey: target.repository.repositoryKey,
        checkoutPath,
        recommendations: cachedRecommendRulePacks(checkoutPath)
          .map(({ pack, ruleCount, reason, folder }) => ({ packId: pack.id, ruleCount, reason, folder })),
      };
    },
  }),
  defineOperation({
    id: "rerail_rules_list",
    method: "GET",
    path: "/api/rerail-rules",
    summary: "List the By Rerail rules that ship with Rerail, each with the pack it belongs to (null for rules in no pack).",
    query: kindFilter,
    output: z.object({ rules: z.array(rerailRuleSchema) }),
    handler: (stores, input) => {
      const bundle = stores.bundle();
      return {
        rules: [...bundle.documents.values()]
          .filter(document => input.kind === undefined || document.kind === input.kind)
          .map(document => document.kind === "file"
            ? { kind: "file" as const, rule: document.rule, packId: bundle.packOf.get(document.rule.id) ?? null, ...inactiveOf("file", document.rule) }
            : { kind: "conversation" as const, rule: document.rule, packId: bundle.packOf.get(document.rule.id) ?? null }),
      };
    },
  }),
  defineOperation({
    id: "library_rules_list",
    method: "GET",
    path: "/api/library-rules",
    summary: "List the rules created by you (the library). Library rules run only where they are added to a repository.",
    query: kindFilter,
    output: z.object({
      rules: z.array(libraryRuleRecordSchema),
      quarantined: z.array(quarantinedRuleSchema).describe("Stored rules that fail validation; they never run."),
    }),
    handler: (stores, input) => ({
      rules: listLibraryRules(stores.database(), input.kind === undefined ? {} : { kind: input.kind }).map(libraryRecord),
      quarantined: quarantinedLibraryRules(stores.database()).filter(entry => input.kind === undefined || entry.kind === input.kind),
    }),
  }),
  defineOperation({
    id: "library_rules_get",
    method: "GET",
    path: "/api/library-rules/{id}",
    summary: "Fetch one rule created by you.",
    params: libraryRuleId,
    output: z.object({ rule: libraryRuleRecordSchema }),
    handler: (stores, input) => {
      const rule = getLibraryRule(stores.database(), input.id);
      if (rule === undefined) throw new ApiError(404, "rule-not-found", `library rule ${input.id} does not exist`);
      return { rule: libraryRecord(rule) };
    },
  }),
  defineOperation({
    id: "library_rules_upsert",
    method: "PUT",
    path: "/api/library-rules/{id}",
    summary: "Create or replace a rule created by you; a file rule is compiled in the kernel first. With attach: true it is also added to the target repository in folder, setting the repository up if it is not yet. Writing the id of a By Rerail rule that is not also in your library stores an edited copy instead, added wherever the original is in the target repository, with the original switched off there.",
    params: libraryRuleId,
    body: libraryUpsertBodySchema,
    output: libraryUpsertResultSchema,
    handler: (stores, input, caller) => {
      if (input.rule.id !== input.id) throw new ApiError(400, "rule-id-mismatch", `path id ${input.id} does not match rule.id ${input.rule.id}`);
      const db = stores.database();
      const bundle = stores.bundle();
      const existing = db.select({ copiedFrom: libraryRules.copiedFromRuleId }).from(libraryRules).where(eq(libraryRules.ruleId, input.id)).get();
      // A library rule a later release bundled the id of wins over the By Rerail rule, so writing it edits it.
      if (existing === undefined && bundle.documents.has(input.id)) {
        // Copy-on-edit: an agent may add the copy to a set-up repository, as people may.
        const target = resolveTarget(stores, input, caller, true);
        const copied = libraryWrite(() => copyRerailRuleToLibrary(db, bundle, target.repository.repositoryKey, input.id, input.rule));
        return {
          rule: libraryRecord(copied.rule),
          copiedFromRuleId: input.id,
          attached: { repositoryKey: target.repository.repositoryKey, folders: [...copied.folders] },
        };
      }
      // An agent editing an existing rule changes it in every repository that runs it, so each set-up one must allow
      // it. Edits to a copy of a By Rerail rule are allowed as making the copy was; a rule the agent creates in
      // this call may be added where it likes.
      if (caller.kind === "agent" && existing !== undefined && existing.copiedFrom === null) guardRepositoriesUsing(stores, caller, input.id);
      const target = input.attach === true ? resolveTarget(stores, input, caller, true) : undefined;
      if (target !== undefined) guardAgent(stores, caller, target, existing === undefined || existing.copiedFrom !== null);
      const document = libraryWrite(() => validateLibraryDocument(input.rule, input.kind));
      // The rule, its attachment and a first set-up are one change.
      libraryWrite(() => db.transaction(tx => {
        writeLibraryRule(tx, bundle, document, input.revision === undefined ? {} : { revision: input.revision });
        if (target === undefined) return;
        const repositoryKey = target.repository.repositoryKey;
        const attachment = { folder: target.folder, sourceKind: "library_rule" as const, sourceId: input.id };
        if (notSetUp(tx, repositoryKey)) recordSetUp(tx, repositoryKey, { packs: [], rules: [attachment] });
        else addAttachment(tx, { repositoryKey, ...attachment });
      }, { behavior: "immediate" }));
      const rule = getLibraryRule(db, input.id);
      if (rule === undefined) throw new ApiError(500, "internal", `library rule ${input.id} was not stored`);
      if (target === undefined) return { rule: libraryRecord(rule), copiedFromRuleId: null, attached: null };
      return { rule: libraryRecord(rule), copiedFromRuleId: null, attached: { repositoryKey: target.repository.repositoryKey, folders: [target.folder] } };
    },
  }),
  defineOperation({
    id: "library_rules_remove",
    method: "DELETE",
    path: "/api/library-rules/{id}",
    summary: "Delete a rule created by you, with every repository's attachments and settings for it. A coding agent can't delete a rule a set-up repository uses.",
    params: libraryRuleId,
    output: z.object({ removed: z.literal(1) }),
    handler: (stores, input, caller) => {
      const db = stores.database();
      if (caller.kind === "agent") guardRepositoriesUsing(stores, caller, input.id);
      if (!removeLibraryRule(db, input.id)) throw new ApiError(404, "rule-not-found", `library rule ${input.id} does not exist`);
      return { removed: 1 as const };
    },
  }),
];

/** Refuse an agent's change to a library rule that any set-up repository has added. */
function guardRepositoriesUsing(stores: RerailApiStores, caller: ApiCaller, ruleId: string): void {
  const db = stores.database();
  const users = db.selectDistinct({ key: ruleAttachments.repositoryKey }).from(ruleAttachments)
    .where(and(eq(ruleAttachments.sourceKind, "library_rule"), eq(ruleAttachments.sourceId, ruleId))).all();
  for (const { key } of users) guardAgent(stores, caller, { repository: requireRepository(db, key), checkout: undefined, folder: "" });
}

/**
 * The rules `rules_activity` reports: those present in the repository, or without one every By Rerail rule and
 * every rule created by you. An unknown repository has none.
 */
export function activityRules(stores: RerailApiStores, key: string | undefined): { id: string; kind: "file" | "conversation"; rule: unknown }[] {
  const db = stores.database();
  const bundle = stores.bundle();
  if (key === undefined) {
    return [
      ...[...bundle.documents.values()].map(document => ({ id: document.rule.id, kind: document.kind, rule: document.rule as unknown })),
      ...listLibraryRules(db).map(rule => ({ id: rule.document.rule.id, kind: rule.document.kind, rule: rule.document.rule as unknown })),
    ];
  }
  const repository = findRepository(db, key);
  if (repository === undefined) return [];
  const present = new Map(listRepositoryRules(db, bundle, repository.repositoryKey).rules
    .map(rule => [rule.document.rule.id, { kind: rule.document.kind, rule: rule.document.rule as unknown }] as const));
  return [...present].map(([id, { kind, rule }]) => ({ id, kind, rule }));
}

function startOrStop(
  stores: RerailApiStores,
  input: z.infer<typeof startBodySchema>,
  caller: ApiCaller,
  started: boolean,
): z.infer<typeof startResultSchema> {
  if (caller.kind === "agent" && input.scope === "repository") {
    throw new ApiError(403, "agent-not-permitted", "a coding agent starts and stops Rerail only in its own checkout; the whole repository is the dashboard's or the CLI's");
  }
  const target = resolveTarget(stores, input, caller, true);
  const db = stores.database();
  if (input.scope === "repository") setRepositoryStarted(db, target.repository.repositoryKey, started);
  else if (target.checkout === undefined) throw new ApiError(400, "target-required", "scope checkout needs a checkout");
  else {
    guardAgentCheckout(caller, target.checkout);
    setCheckoutStarted(db, target.checkout, started);
  }
  return {
    repository: repositorySummary(requireRepository(db, target.repository.repositoryKey)),
    checkout: target.checkout === undefined ? null : {
      checkoutPath: target.checkout.checkoutPath,
      state: stateOf(effectiveStatus(db, target.checkout)),
    },
  };
}
