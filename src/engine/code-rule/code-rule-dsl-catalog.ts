/**
 * Compiled code rules per checkout. A checkout's catalog compiles the code rules its repository's
 * configuration makes present and places each one per folder (on at a mode, or off), so one native workspace
 * evaluates every folder of the checkout. Executables are compiled once per process and shared by every catalog.
 */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { and, eq, inArray, or } from "drizzle-orm";
import { loadRuleCatalog } from "../catalog/rule-catalog.js";
import { conversationRulesEnabled } from "../experimental-features.js";
import { worktreeRuleCatalog } from "../catalog/worktree-pack.js";
import { physicalPath } from "../catalog/checkout-control.js";
import { CheckoutOwners, type CheckoutOwner } from "../catalog/checkout-owner.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "../storage/database.js";
import { reportQuarantinedRule, type QuarantinedRule } from "../catalog/rule-quarantine.js";
import { languageNotSupportedOf } from "./code-rule-language-support.js";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "../native/native.js";
import type { RepositoryAuditRuleProvenance, AuditRuleProvenance, RulePlacementDecision } from "../audit/audit.js";
import { bundledCodeRuleDslDirectory, relationalRulesOff } from "./code-rule-compile.js";
import {
  folderContains,
  type RuleCatalog,
  type CheckoutContext,
  type CheckoutRuleSet,
  type EffectiveRule,
  type CodeRulePlacement,
  type CodeRulePlacementEntry,
  type RepositoryStatus,
} from "../catalog/repository-model.js";
import { repositoryIdentityLocation, type RepositoryIdentityLocation } from "../catalog/repository-identity.js";
import {
  followsGitignore,
  recordedCheckoutsReader,
  recordedCheckoutsVersionReader,
  recordStartedWorktree,
  type RecordedCheckouts,
  type RecordedCheckoutStatus,
} from "../catalog/repository-registry.js";
import { loadCheckoutRuleSet, ruleConfigurationSignature, ruleConfigurationVersionReader } from "../catalog/rule-configuration.js";
import { LIBRARY_OWNER, quarantinedLibraryRules } from "../catalog/rule-library.js";
import type { ConversationRule } from "../catalog/rule-schema.js";
import type { RerailRule } from "../catalog/rerail-rules/rerail-rule.js";
import { ruleAttachments, ruleSettings } from "../storage/schema.js";
import { compareDeterministicStrings, ruleSourceText, stableJsonText } from "../stable-json.js";

export { bundledCodeRuleDslDirectory };

/** Quarantine owner of a By Rerail rule (in a pack or added alone) the engine rejects. */
export const RERAIL_OWNER = "rerail";

function digest(value: string): string {
  return `sha256:${createHash("sha256").update(value).digest("hex")}`;
}

export interface CodeRuleCapabilities {
  readonly path: string;
  readonly json: string;
  readonly digest: string;
}

/** The capability manifest rules compile against; the bundled one unless a path is given. */
export function loadCodeRuleCapabilities(path: string = resolve(bundledCodeRuleDslDirectory(), "capabilities.json")): CodeRuleCapabilities {
  const json = readFileSync(resolve(path), "utf8");
  return { path: resolve(path), json, digest: digest(json) };
}

type CompileOutcome =
  | { readonly ok: true; readonly executable: NativeCompiledCodeRuleDsl; readonly crossFile: boolean }
  | { readonly ok: false; readonly reason: string };

/** A compiled outcome as the cache keeps it: a failure whole, an executable only while a catalog still holds it. */
type HeldOutcome =
  | { readonly ok: true; readonly executable: WeakRef<NativeCompiledCodeRuleDsl>; readonly crossFile: boolean }
  | { readonly ok: false; readonly reason: string };

/**
 * Compiled executables shared by every checkout of the process: one per rule id, capabilities and `config` (a
 * repository's option values), for the source last compiled. Editing a rule replaces its entries, so a long-running
 * daemon keeps no earlier versions; two repositories configuring one rule differently each keep theirs.
 *
 * An executable is held weakly: the catalogs of the checkouts that run it hold it, and once the daemon closes the last
 * runtime using it (`IDLE_RUNTIME_MS` after its repository's last session ended) the engine frees its program. Most
 * rules compile to 0.3 MB, the Node pack's builtins rule to 512 MB, so keeping every rule ever compiled kept a daemon
 * with no session open at its busiest size.
 */
export class CompiledRuleCache {
  readonly #outcomes = new Map<string, { readonly sourceDigest: string; readonly outcome: HeldOutcome }>();

  /** Rules (per capabilities and config) with a compiled version known. */
  get size(): number {
    return this.#outcomes.size;
  }

  /** `config` is the stable JSON of the rule's option values (`null` for none): it names the variant. */
  compile(capabilities: CodeRuleCapabilities, policyId: string, sourceText: string, sourceDigest: string, config: string): CompileOutcome {
    const key = `${capabilities.digest}\0${policyId}\0${config}`;
    const cached = this.#outcomes.get(key);
    const held = cached?.sourceDigest === sourceDigest ? liveOutcome(cached.outcome) : undefined;
    if (held !== undefined) return held;
    const outcome = compileOutcome(capabilities, policyId, sourceText);
    this.#outcomes.set(key, { sourceDigest, outcome: outcome.ok ? { ...outcome, executable: new WeakRef(outcome.executable) } : outcome });
    return outcome;
  }
}

/** The outcome `held` keeps, or undefined once its executable was freed. */
function liveOutcome(held: HeldOutcome): CompileOutcome | undefined {
  if (!held.ok) return held;
  const executable = held.executable.deref();
  return executable === undefined ? undefined : { ...held, executable };
}

function compileOutcome(capabilities: CodeRuleCapabilities, policyId: string, sourceText: string): CompileOutcome {
  try {
    const executable = loadNativeBinding().CodeRuleDslRuntime.compile(sourceText, "json", capabilities.json);
    const canonical = JSON.parse(executable.canonicalJson()) as { id?: unknown };
    return canonical.id === policyId
      ? { ok: true, executable, crossFile: executable.readsOtherFiles() }
      : { ok: false, reason: `Compiled policy ID does not match rule document: ${policyId}` };
  } catch (error) {
    return { ok: false, reason: error instanceof Error ? error.message : String(error) };
  }
}

const compiledRules = new CompiledRuleCache();

export interface CompiledCodeRuleDslEntry {
  readonly id: string;
  readonly executable: NativeCompiledCodeRuleDsl;
  /** Where the code rule runs in the checkout, at which mode, and where suppression comments are ignored; passed to the native workspace. */
  readonly placement: CodeRulePlacement;
  /** Whether its finding in one file can depend on another file's text, as the engine routes its clauses (`readsOtherFiles`). */
  readonly crossFile: boolean;
}

/**
 * A rule written in `.rerail` that the checkout runs, placed per folder as a compiled rule is. The `.rerail` runner
 * runs it over the files on disk (`src/engine/rr/`); the native workspace never holds it.
 */
export interface RerailRuleEntry {
  readonly id: string;
  /** The rule's file, installed in its pack. */
  readonly path: string;
  /** The files the rule reads, as its `.scope` says: some include glob (every file when none), and no exclude glob. */
  readonly scope: { readonly include: readonly string[]; readonly exclude: readonly string[] };
  /** The language its `.language` names, or null. */
  readonly language: string | null;
  /**
   * The rule file, then every file it includes, each with the SHA-256 of the bytes the catalog read: a run checks
   * them first, since the runner reads the files from disk.
   */
  readonly files: readonly { readonly path: string; readonly sha256: string }[];
  readonly placement: CodeRulePlacement;
  readonly provenance: RepositoryAuditRuleProvenance;
}

/** Immutable provenance and executables used to construct one native evaluation workspace. */
export interface CompiledCodeRuleDslSnapshot {
  readonly version: string;
  readonly codeRules: readonly CompiledCodeRuleDslEntry[];
  /** The provenance of `codeRules`. */
  readonly provenance: readonly AuditRuleProvenance[];
  /** The rules written in `.rerail` the checkout runs, each with its own provenance. */
  readonly rerailRules: readonly RerailRuleEntry[];
}

export interface CodeRuleDslCatalogStatus {
  /** Code rules placed somewhere in the checkout: those compiled, and those written in `.rerail`. */
  readonly ruleCount: number;
  /** Digest of the placed set: capabilities, and each code rule's id, source, source digest and placement. */
  readonly version: string;
  /** Source identity captured before loading; never advanced by presentation reads or failed compilation. */
  readonly sourceSignature: string;
}

/** What a native code-rule workspace needs from a catalog. */
export interface CodeRuleDslCatalog {
  readonly status: CodeRuleDslCatalogStatus;
  snapshot(): CompiledCodeRuleDslSnapshot;
  provenance(): readonly AuditRuleProvenance[];
  /** Rules the last load skipped because they fail validation or compilation. */
  quarantined(): readonly QuarantinedRule[];
  /** Reload when the source signature changed; returns whether the compiled set differs. */
  refresh(): boolean;
}

/** One load of a checkout's rules. */
export interface LoadedCheckoutRules {
  /** The checkout's configuration; undefined when it may run nothing (unrecorded). */
  readonly ruleSet: CheckoutRuleSet | undefined;
  /** Every folder where presence, `on`, `mode` or `suppressions` can change: attachment and setting folders, and `""`. */
  readonly folders: readonly string[];
  /** Library rules this checkout adds that fail validation. */
  readonly quarantined: readonly QuarantinedRule[];
}

/** Where a checkout catalog reads its rules from. */
export interface CheckoutRuleSource {
  /** Where quarantined rules are reported as living. */
  readonly label: string;
  /** Changes whenever `load()` could return something else; cheaper than a load. */
  signature(): string;
  load(): LoadedCheckoutRules;
}

interface PlacedRule {
  readonly rule: EffectiveRule;
  readonly decisions: RulePlacementDecision[];
}

/** Each code rule present in some folder, with how it runs in every folder it is present in. */
function placedRules(ruleSet: CheckoutRuleSet, folders: readonly string[]): Map<string, PlacedRule> {
  const placed = new Map<string, PlacedRule>();
  for (const folder of folders) {
    for (const rule of ruleSet.rulesFor(folder)) {
      const id = rule.document.rule.id;
      let entry = placed.get(id);
      if (entry === undefined) {
        entry = { rule, decisions: [] };
        placed.set(id, entry);
      }
      entry.decisions.push({
        folder, on: rule.on, mode: rule.mode, onFrom: rule.decidedBy.on, modeFrom: rule.decidedBy.mode,
        suppressions: rule.suppressions, suppressionsFrom: rule.decidedBy.suppressions,
      });
    }
  }
  return placed;
}

/**
 * The native placement: per folder the mode (or null, off; a folder the rule is absent from is off) and whether
 * suppression comments are ignored there, keeping only folders that differ from the deepest kept folder containing
 * them. Where the rule is off, suppressions are left as the parent has them. A rule off everywhere has no entries.
 */
function placementOf(decisions: readonly RulePlacementDecision[], folders: readonly string[]): CodeRulePlacement {
  const kept: CodeRulePlacementEntry[] = [];
  for (const folder of folders) {
    const decision = decisions.find(entry => entry.folder === folder);
    const mode = decision?.on === true ? decision.mode : null;
    let parent: CodeRulePlacementEntry | undefined;
    for (const entry of kept) {
      if (folderContains(entry.folder, folder) && (parent === undefined || entry.folder.length > parent.folder.length)) parent = entry;
    }
    const ignore = mode === null ? parent?.suppressions === "ignore" : decision?.suppressions === "ignore";
    const entry: CodeRulePlacementEntry = ignore ? { folder, mode, suppressions: "ignore" } : { folder, mode };
    const differs = parent === undefined ? mode !== null : parent.mode !== mode || parent.suppressions !== entry.suppressions;
    if (differs) kept.push(entry);
  }
  return kept;
}

interface CatalogState {
  readonly status: CodeRuleDslCatalogStatus;
  readonly snapshot: CompiledCodeRuleDslSnapshot;
  readonly quarantined: readonly QuarantinedRule[];
}

/**
 * One checkout's code rules, compiled into resident executables and placed per folder. `refresh()` reloads only
 * when the source signature changes; an unchanged rule is never recompiled.
 */
export class CheckoutCodeRuleCatalog implements CodeRuleDslCatalog {
  readonly #source: CheckoutRuleSource;
  readonly #capabilities: CodeRuleCapabilities;
  #state: CatalogState;

  constructor(source: CheckoutRuleSource, capabilities: CodeRuleCapabilities = loadCodeRuleCapabilities()) {
    this.#source = source;
    this.#capabilities = capabilities;
    const sourceSignature = source.signature();
    this.#state = this.#build(source.load(), sourceSignature);
  }

  get status(): CodeRuleDslCatalogStatus {
    return this.#state.status;
  }

  snapshot(): CompiledCodeRuleDslSnapshot {
    return this.#state.snapshot;
  }

  provenance(): readonly AuditRuleProvenance[] {
    return this.#state.snapshot.provenance;
  }

  quarantined(): readonly QuarantinedRule[] {
    return this.#state.quarantined;
  }

  refresh(): boolean {
    const sourceSignature = this.#source.signature();
    if (sourceSignature === this.#state.status.sourceSignature) return false;
    const previous = this.#state.status.version;
    this.#state = this.#build(this.#source.load(), sourceSignature);
    return this.#state.status.version !== previous;
  }

  #build(loaded: LoadedCheckoutRules, sourceSignature: string): CatalogState {
    const quarantined = [...loaded.quarantined];
    const built: BuiltRule[] = [];
    const rerail: BuiltRerailRule[] = [];
    const placed = loaded.ruleSet === undefined ? new Map<string, PlacedRule>() : placedRules(loaded.ruleSet, loaded.folders);
    for (const id of [...placed.keys()].sort(compareDeterministicStrings)) {
      const compiled = this.#compiled(id, placed.get(id)!, loaded);
      if (compiled === undefined) continue;
      if ("quarantined" in compiled) quarantined.push(compiled.quarantined);
      else if ("rerail" in compiled) rerail.push(compiled);
      else built.push(compiled);
    }
    return catalogState({ capabilitiesDigest: this.#capabilities.digest, label: this.#source.label, sourceSignature }, { built, rerail }, quarantined);
  }

  /**
   * One placed rule compiled: its entry, provenance and version line; a rule written in `.rerail`, kept for its runner;
   * why it is quarantined; undefined where it is placed nowhere.
   */
  #compiled(id: string, { rule, decisions }: PlacedRule, loaded: LoadedCheckoutRules): BuiltRule | BuiltRerailRule | { readonly quarantined: QuarantinedRule } | undefined {
    const placement = placementOf(decisions, loaded.folders);
    if (placement.length === 0) return undefined;
    const refused = refusedBeforeCompile(id, rule);
    if (refused !== undefined) return { quarantined: refused };
    if (rule.document.kind === "rerail") return rerailBuilt(loaded.ruleSet!, { id, rule, decisions, placement }, rule.document.rule);
    const source = ruleSourceText(rule.document.rule);
    if (source === undefined) throw new Error(`Policy ${id} is not JSON representable`);
    const outcome = compiledRules.compile(this.#capabilities, id, source.text, source.digest, stableJsonText(rule.config) ?? "");
    if (!outcome.ok) return { quarantined: refusedRule(id, rule, outcome.reason) };
    const checkout = loaded.ruleSet!.checkout;
    return {
      entry: { id, executable: outcome.executable, placement, crossFile: outcome.crossFile },
      provenance: {
        ruleId: id, revision: rule.revision, payloadHash: rule.payloadHash, source: { ...rule.source },
        repositoryKey: checkout.repositoryKey, checkoutPath: checkout.checkoutPath, placement: decisions,
      },
      versioned: { id, sourceDigest: source.digest, placement, source: rule.source },
    };
  }
}

/** A rule the catalog compiled: its entry, its provenance, and its line of the catalog's version. */
interface BuiltRule {
  readonly entry: CompiledCodeRuleDslEntry;
  readonly provenance: RepositoryAuditRuleProvenance;
  readonly versioned: unknown;
}

/** A rule written in `.rerail` the catalog keeps for the `.rerail` runner, and its line of the catalog's version. */
interface BuiltRerailRule {
  readonly rerail: RerailRuleEntry;
  readonly versioned: unknown;
}

/** A placed rule written in `.rerail`, kept with its placement; its line of the version digests its content. */
function rerailBuilt(
  ruleSet: CheckoutRuleSet,
  { id, rule, decisions, placement }: { readonly id: string; readonly rule: EffectiveRule; readonly decisions: RulePlacementDecision[]; readonly placement: CodeRulePlacement },
  document: RerailRule,
): BuiltRerailRule {
  const { repositoryKey, checkoutPath } = ruleSet.checkout;
  return {
    rerail: {
      id, path: document.path, scope: document.scope, language: document.language, placement,
      files: [{ path: document.path, sha256: document.sha256 }, ...document.includes],
      provenance: { ruleId: id, revision: rule.revision, payloadHash: rule.payloadHash, source: { ...rule.source }, repositoryKey, checkoutPath, placement: decisions },
    },
    versioned: { id, kind: "rerail", sourceDigest: rule.payloadHash, placement, source: rule.source },
  };
}

/**
 * Why a placed rule is quarantined before it compiles: the repository's config for it does not apply, or it is a
 * relational rule while relational rules are off. Undefined when it goes on to compile.
 */
function refusedBeforeCompile(id: string, rule: EffectiveRule): QuarantinedRule | undefined {
  if (rule.configError !== null)
    return { ruleId: id, kind: "code", owner: RERAIL_OWNER, reason: `The repository's config for this rule does not apply: ${rule.configError}` };
  const relationalOff = rule.document.kind === "code" ? relationalRulesOff(rule.document.rule) : undefined;
  if (relationalOff === undefined) return undefined;
  return { ruleId: id, kind: "code", owner: rule.source.kind === "library_rule" ? LIBRARY_OWNER : RERAIL_OWNER, reason: relationalOff };
}

/**
 * A rule the engine refused. One naming a language the engine does not read yet is inactive by design: quarantined
 * with that code, not reported.
 */
function refusedRule(id: string, rule: EffectiveRule, reason: string): QuarantinedRule {
  const unsupported = languageNotSupportedOf(reason);
  return {
    ruleId: id, kind: "code", owner: rule.source.kind === "library_rule" ? LIBRARY_OWNER : RERAIL_OWNER,
    reason: unsupported?.reason ?? reason,
    ...(unsupported === undefined ? {} : { code: unsupported.code }),
  };
}

/** The catalog's state from the rules it built and those it quarantined, each quarantined one without a code reported. */
function catalogState(
  catalog: { readonly capabilitiesDigest: string; readonly label: string; readonly sourceSignature: string },
  { built, rerail }: { readonly built: readonly BuiltRule[]; readonly rerail: readonly BuiltRerailRule[] },
  quarantined: readonly QuarantinedRule[],
): CatalogState {
  for (const entry of quarantined) if (entry.code === undefined) reportQuarantinedRule(catalog.label, entry);
  const policies = [...built.map(rule => rule.versioned), ...rerail.map(rule => rule.versioned)];
  const canonical = stableJsonText({ capabilitiesDigest: catalog.capabilitiesDigest, format: "rerail.policy-dsl-catalog/v3", policies });
  if (canonical === undefined) throw new Error("Policy catalog version is not JSON representable");
  const codeRules = built.map(rule => rule.entry);
  const rerailRules = rerail.map(rule => rule.rerail);
  return {
    status: { ruleCount: codeRules.length + rerailRules.length, version: digest(canonical), sourceSignature: catalog.sourceSignature },
    snapshot: { version: digest(canonical), codeRules, provenance: built.map(rule => rule.provenance), rerailRules },
    quarantined,
  };
}

/** Whether a checkout may run rules: `ok` only when it is recorded, effectively started, and its repository set up. */
export type CheckoutEligibility = "ok" | "stopped" | "not-set-up" | "unrecorded";

/** A source of conversation rules that can tell cheaply whether they changed. */
export interface ConversationRuleSource {
  signature(): string;
  /** The conversation rules that are on, each at its effective mode. */
  rules(): readonly ConversationRule[];
  close(): void;
}

interface CheckoutEntry {
  readonly root: string;
  location: RepositoryIdentityLocation | undefined;
  checkout: CheckoutContext | undefined;
  parents: readonly string[];
  status: RepositoryStatus | undefined;
  eligibility: CheckoutEligibility;
  catalog: CheckoutCodeRuleCatalog | undefined;
}

/**
 * The checkouts one runtime evaluates, each with its eligibility and catalog, over one database connection.
 * Eligibility and configuration are read again on `refresh`, so a stop, start or settings change made elsewhere
 * reaches open sessions on their next event.
 */
export class CheckoutCatalogs {
  readonly databasePath: string;
  readonly database: RerailDatabase;
  readonly #ruleCatalog: RuleCatalog | undefined;
  readonly #capabilities: CodeRuleCapabilities;
  readonly #entries = new Map<string, CheckoutEntry>();
  readonly #recordedCheckoutsVersion: () => string;
  readonly #recorded: RecordedCheckouts;
  readonly #configurationVersion: () => string;

  /**
   * `options.ruleCatalog` fixes the installed packs; without it each read follows them (`loadRuleCatalog`). Either way a
   * checkout's own rules are added to its catalog.
   */
  constructor(databasePath: string = defaultRerailDatabasePath(), options: { capabilities?: CodeRuleCapabilities; ruleCatalog?: RuleCatalog } = {}) {
    this.databasePath = resolve(databasePath);
    this.database = openRerailDatabase(this.databasePath);
    this.#ruleCatalog = options.ruleCatalog;
    this.#capabilities = options.capabilities ?? loadCodeRuleCapabilities();
    this.#recordedCheckoutsVersion = recordedCheckoutsVersionReader(this.database);
    this.#recorded = recordedCheckoutsReader(this.database);
    this.#configurationVersion = ruleConfigurationVersionReader(this.database);
  }

  /**
   * The catalog `root`'s rules come from: the installed packs and the checkout's own rules (`worktree-pack.ts`). A pack
   * installed or removed, or a file in the checkout's `.rerail/rules` changed, changes its digest, and with it the
   * checkout's configuration signature, so the next refresh rebuilds its rules.
   */
  #ruleCatalogOf(root: string): RuleCatalog {
    return worktreeRuleCatalog(this.#ruleCatalog ?? loadRuleCatalog(), root);
  }

  /** The checkout at `root` (a physical checkout top level) as last read; refreshed with `refresh`. */
  checkout(root: string): CheckoutContext | undefined {
    return this.#entry(root).checkout;
  }

  eligibility(root: string): CheckoutEligibility {
    return this.#entry(root).eligibility;
  }

  /** Whether audits in `root` leave out files Git ignores: its repository's setting, on for a checkout not recorded. */
  followsGitignore(root: string): boolean {
    return followsGitignore(this.database, this.#entry(root).checkout);
  }

  /** The checkout and its recorded parent checkouts, whose settings apply in it. */
  ownerCheckoutPaths(root: string): readonly string[] {
    const entry = this.#entry(root);
    return entry.checkout === undefined ? [] : [entry.checkout.checkoutPath, ...entry.parents];
  }

  /** Re-read `root`'s record, start state and set-up; returns its eligibility. */
  refresh(root: string): CheckoutEligibility {
    const entry = this.#entry(root, false);
    this.#read([entry]);
    return entry.eligibility;
  }

  /**
   * `refresh` of every one of `roots`, the recorded ones read in one statement: what a session that knows every worktree
   * of its repository asks at every event. Returns each root's eligibility.
   */
  refreshAll(roots: Iterable<string>): Map<string, CheckoutEligibility> {
    const entries = [...roots].map(root => this.#entry(root, false));
    this.#read(entries);
    return new Map(entries.map(entry => [entry.root, entry.eligibility]));
  }

  /**
   * Every repository's rule configuration as one text, read in one statement (`ruleConfigurationVersionReader`): when it
   * differs from the last one, some checkout's rules may read differently, whichever checkout a process configured.
   */
  configurationVersion(): string {
    return this.#configurationVersion();
  }

  /** See {@link recordedCheckoutsVersion}; `CheckoutOwners` forgets its cached ownership when it changes. */
  recordedCheckoutsVersion(): string {
    return this.#recordedCheckoutsVersion();
  }

  /**
   * The recorded checkouts strictly inside `root` (physical paths), at any depth, without a read of every row: the
   * session's ignore rules ask this at every tool call (`RecordedCheckouts.within`).
   */
  recordedCheckoutsWithin(root: string): string[] {
    return this.#recorded.within(root);
  }

  /** The compiled code rules of `root`; its `refresh()` re-reads eligibility and configuration. */
  catalog(root: string): CheckoutCodeRuleCatalog {
    const entry = this.#entry(root);
    entry.catalog ??= new CheckoutCodeRuleCatalog({
      label: this.databasePath,
      signature: () => this.#signature(entry),
      load: () => this.#load(entry),
    }, this.#capabilities);
    return entry.catalog;
  }

  /**
   * The conversation rules of the session folder `folder` (relative to `root`), as they change. Internal testing: none
   * while conversation rules are off in this process ({@link conversationRulesEnabled}), wherever they are stored.
   */
  conversationRules(root: string, folder: string): ConversationRuleSource {
    const entry = this.#entry(root);
    return {
      signature: () => this.#signature(entry),
      rules: () => {
        if (!conversationRulesEnabled()) return [];
        const ruleSet = this.#ruleSet(entry);
        if (ruleSet === undefined || entry.eligibility !== "ok") return [];
        return ruleSet.conversationRules(folder).filter(rule => rule.on).map(rule => rule.document.rule as ConversationRule);
      },
      close: () => {},
    };
  }

  close(): void {
    this.#entries.clear();
    this.database.$client.close();
  }

  #entry(root: string, read = true): CheckoutEntry {
    let entry = this.#entries.get(root);
    if (entry === undefined) {
      entry = {
        root, location: undefined, checkout: undefined,
        parents: [], status: undefined, eligibility: "unrecorded", catalog: undefined,
      };
      this.#entries.set(root, entry);
      if (read) this.#read([entry]);
    }
    return entry;
  }

  /**
   * Only a recorded checkout is eligible; its row names its repository, so Git is asked once per checkout. The recorded
   * ones are read in one statement, with their parents and start state (`RecordedCheckouts.read`). A worktree of a
   * started clone that no session started in is recorded here, the first time a path in it is looked up.
   */
  #read(entries: readonly CheckoutEntry[]): void {
    const roots = entries.map(entry => entry.root);
    let recorded = this.#recorded.read(roots);
    // A worktree recorded here may hold another of the entries, so all of them are read again with its row.
    if (entries.filter(entry => !recorded.has(entry.root) && this.#recordWorktree(entry)).length > 0) recorded = this.#recorded.read(roots);
    for (const entry of entries) this.#apply(entry, recorded.get(entry.root));
  }

  #apply(entry: CheckoutEntry, recorded: RecordedCheckoutStatus | undefined): void {
    if (recorded === undefined) {
      entry.checkout = undefined;
      entry.parents = [];
      entry.status = undefined;
      entry.eligibility = "unrecorded";
      return;
    }
    entry.location ??= repositoryIdentityLocation(entry.root);
    const { git, main } = entry.location;
    const { status } = recorded;
    entry.checkout = { checkoutPath: entry.root, repositoryKey: recorded.repositoryKey, cloneId: recorded.cloneId, git, main };
    entry.parents = recorded.parents;
    entry.status = status;
    if (!status.started) entry.eligibility = "stopped";
    else entry.eligibility = status.setUp ? "ok" : "not-set-up";
  }

  /** Records `entry` when it is a worktree of a started clone (`recordStartedWorktree`); whether it did. */
  #recordWorktree(entry: CheckoutEntry): boolean {
    // A folder outside Git may become a checkout, so only a Git location is kept.
    const location = entry.location ?? repositoryIdentityLocation(entry.root);
    if (location.git) entry.location = location;
    return recordStartedWorktree(this.database, entry.root, location);
  }

  /** `<eligibility>` while unrecorded, else `<eligibility>\0<configuration signature>`; see `configurationSignatureOf`. */
  #signature(entry: CheckoutEntry): string {
    this.#read([entry]);
    if (entry.checkout === undefined) return entry.eligibility;
    const signature = ruleConfigurationSignature(this.database, this.#ruleCatalogOf(entry.root), entry.checkout.repositoryKey, this.ownerCheckoutPaths(entry.root));
    return `${entry.eligibility}\0${signature}`;
  }

  #ruleSet(entry: CheckoutEntry): CheckoutRuleSet | undefined {
    if (entry.checkout === undefined || entry.status === undefined) return undefined;
    return loadCheckoutRuleSet(this.database, this.#ruleCatalogOf(entry.root), entry.checkout, entry.status, entry.parents);
  }

  #load(entry: CheckoutEntry): LoadedCheckoutRules {
    const ruleSet = this.#ruleSet(entry);
    if (ruleSet === undefined || entry.eligibility !== "ok") return { ruleSet, folders: [""], quarantined: [] };
    const key = ruleSet.checkout.repositoryKey;
    const owners = this.ownerCheckoutPaths(entry.root);
    const attachmentFolders = this.database.selectDistinct({ folder: ruleAttachments.folder }).from(ruleAttachments)
      .where(eq(ruleAttachments.repositoryKey, key)).all();
    const settingFolders = this.database.selectDistinct({ folder: ruleSettings.folder }).from(ruleSettings)
      .where(or(
        and(eq(ruleSettings.ownerKind, "repository"), eq(ruleSettings.ownerId, key)),
        and(eq(ruleSettings.ownerKind, "checkout"), inArray(ruleSettings.ownerId, [...owners])),
      )).all();
    const folders = [...new Set(["", ...attachmentFolders.map(row => row.folder), ...settingFolders.map(row => row.folder)])]
      .sort(compareDeterministicStrings);
    const missingLibrary = new Set(ruleSet.unavailable
      .filter(reference => reference.sourceKind === "library_rule" && reference.reason === "missing")
      .map(reference => reference.sourceId));
    const quarantined = quarantinedLibraryRules(this.database).filter(rule => rule.kind === "code" && missingLibrary.has(rule.ruleId));
    return { ruleSet, folders, quarantined };
  }
}

/**
 * The checkout a session starts in and its folder there: the owner of its working folder, else the folder itself
 * (a folder outside Git that is not recorded yet).
 */
export function startingCheckout(owners: CheckoutOwners, cwd: string): CheckoutOwner {
  return owners.ownerOf(cwd) ?? { checkoutRoot: physicalPath(cwd), path: "" };
}

/** The conversation rules of a session starting in `cwd`, over a connection of their own that `close()` releases. */
export function openCheckoutConversationRules(databasePath: string, cwd: string): ConversationRuleSource {
  const catalogs = new CheckoutCatalogs(databasePath);
  const start = startingCheckout(new CheckoutOwners(), cwd);
  const source = catalogs.conversationRules(start.checkoutRoot, start.path);
  return { signature: source.signature, rules: source.rules, close: () => catalogs.close() };
}

/**
 * The repository configuration signature a checkout source signature (a catalog's `sourceSignature`, a conversation
 * source's `signature()`) was taken at; undefined for an unrecorded checkout.
 */
export function configurationSignatureOf(sourceSignature: string): string | undefined {
  const separator = sourceSignature.indexOf("\0");
  return separator < 0 ? undefined : sourceSignature.slice(separator + 1);
}
