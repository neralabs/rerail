/**
 * Compiled code rules per checkout. A checkout's catalog compiles the code rules its repository's
 * configuration makes present and places each one per folder (on at a mode, or off), so one native workspace
 * evaluates every folder of the checkout. Executables are compiled once per process and shared by every catalog.
 */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { and, desc, eq, inArray, or } from "drizzle-orm";
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
import { audits, checkouts, ruleAttachments, ruleSettings } from "../storage/schema.js";
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

export type CompileOutcome =
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
  /** The compiles asked of `compileOffThread` that have not ended, by key, each for the source it was asked for. */
  readonly #compiling = new Map<string, Compiling>();
  /** Compiles a tool call waits for. */
  readonly #now = new CompileLine();
  /** Compiles of rules built before any call asks for them: never ahead of one a call waits for. */
  readonly #ahead = new CompileLine();

  /** Rules (per capabilities and config) with a compiled version known. */
  get size(): number {
    return this.#outcomes.size;
  }

  /** `config` is the stable JSON of the rule's option values (`null` for none): it names the variant. */
  compile(capabilities: CodeRuleCapabilities, policyId: string, sourceText: string, sourceDigest: string, config: string): CompileOutcome {
    const key = compileKey(capabilities, policyId, config);
    const held = this.#held(key, sourceDigest);
    if (held !== undefined) return held;
    const outcome = compileOutcome(capabilities, policyId, sourceText);
    this.#keep(key, sourceDigest, outcome);
    return outcome;
  }

  /**
   * `compile`, with the engine's compile run on a thread of Node's worker pool: the JS thread, which answers every
   * session's hooks, runs other work meanwhile. One rule compiles at a time on each line, so the pool keeps threads for
   * file reads; `aheadFor`, the open sessions a catalog builds the rule for before any call asks (a set that grows as
   * sessions join that build), puts it on the line of such rules, and a later ask without it moves it to the other line
   * if it has not started. A rule asked for twice compiles once. A rule only closed sessions asked ahead for, still
   * waiting there, is taken out (`releaseAhead`), and this rejects.
   */
  async compileOffThread(
    capabilities: CodeRuleCapabilities,
    rule: { readonly policyId: string; readonly sourceText: string; readonly sourceDigest: string; readonly config: string },
    aheadFor?: Set<object>,
  ): Promise<CompileOutcome> {
    const key = compileKey(capabilities, rule.policyId, rule.config);
    const known = this.#held(key, rule.sourceDigest) ?? this.#joined(key, rule.sourceDigest, aheadFor);
    if (known !== undefined) return await known;
    return await this.#queued(key, capabilities, rule, aheadFor);
  }

  /**
   * Takes `user`, a session that closed, out of the sessions each rule is built ahead for, and out of the line of rules
   * built ahead each one that has not started and that no open session is left to ask for, so it never compiles for
   * nobody; its asks reject.
   */
  releaseAhead(user: object): void {
    for (const [key, compiling] of this.#compiling) {
      if (compiling.askers === undefined || stillAsked(compiling.askers, user) || !this.#ahead.take(compiling.job)) continue;
      this.#compiling.delete(key);
      compiling.drop();
    }
  }

  /**
   * The compile of this source already asked for: an ask without `aheadFor` moves it to the line of calls if it has not
   * started, and keeps it from being taken out; one with it adds its sessions to those it compiles for.
   */
  #joined(key: string, sourceDigest: string, aheadFor: Set<object> | undefined): Promise<CompileOutcome> | undefined {
    const running = this.#compiling.get(key);
    if (running?.sourceDigest !== sourceDigest) return undefined;
    if (aheadFor !== undefined) {
      running.askers?.add(aheadFor);
      return running.outcome;
    }
    running.askers = undefined;
    if (this.#ahead.take(running.job)) this.#now.add(running.job);
    return running.outcome;
  }

  /** A compile of the rule, added to its line; settles with its outcome once the line has run it. */
  async #queued(
    key: string,
    capabilities: CodeRuleCapabilities,
    rule: { readonly policyId: string; readonly sourceText: string; readonly sourceDigest: string },
    aheadFor: Set<object> | undefined,
  ): Promise<CompileOutcome> {
    const compiled = Promise.withResolvers<CompileOutcome>();
    const job: CompileJob = async () => compiled.resolve(await this.#compileNow(key, capabilities, rule));
    const entry: Compiling = {
      sourceDigest: rule.sourceDigest, outcome: compiled.promise, job, askers: aheadFor === undefined ? undefined : new Set([aheadFor]),
      drop: () => compiled.reject(new Error(`Rule ${rule.policyId} was asked for only by sessions that closed before it compiled`)),
    };
    this.#compiling.set(key, entry);
    (aheadFor === undefined ? this.#now : this.#ahead).add(job);
    try {
      return await compiled.promise;
    } finally {
      if (this.#compiling.get(key) === entry) this.#compiling.delete(key);
    }
  }

  /** The rule's outcome once its line reaches it: one compiled meanwhile on the JS thread is the one kept. */
  async #compileNow(
    key: string,
    capabilities: CodeRuleCapabilities,
    rule: { readonly policyId: string; readonly sourceText: string; readonly sourceDigest: string },
  ): Promise<CompileOutcome> {
    const before = this.#held(key, rule.sourceDigest);
    if (before !== undefined) return before;
    const outcome = await compileOutcomeOffThread(capabilities, rule.policyId, rule.sourceText);
    const meanwhile = this.#held(key, rule.sourceDigest);
    if (meanwhile !== undefined) return meanwhile;
    this.#keep(key, rule.sourceDigest, outcome);
    return outcome;
  }

  #held(key: string, sourceDigest: string): CompileOutcome | undefined {
    const cached = this.#outcomes.get(key);
    return cached?.sourceDigest === sourceDigest ? liveOutcome(cached.outcome) : undefined;
  }

  #keep(key: string, sourceDigest: string, outcome: CompileOutcome): void {
    this.#outcomes.set(key, { sourceDigest, outcome: outcome.ok ? { ...outcome, executable: new WeakRef(outcome.executable) } : outcome });
  }
}

function compileKey(capabilities: CodeRuleCapabilities, policyId: string, config: string): string {
  return `${capabilities.digest}\0${policyId}\0${config}`;
}

/** One compile waiting for its line; it never rejects. */
type CompileJob = () => Promise<void>;

/** A compile asked of `compileOffThread` that has not ended. */
interface Compiling {
  readonly sourceDigest: string;
  readonly outcome: Promise<CompileOutcome>;
  readonly job: CompileJob;
  /**
   * The open sessions of each catalog build that asked for it ahead (`compileOffThread`'s `aheadFor`), while only such
   * asks wait for it; undefined once a call asked for it.
   */
  askers: Set<Set<object>> | undefined;
  /** Rejects `outcome` once the job is taken out of its line (`releaseAhead`), so no ask waits for it. */
  readonly drop: () => void;
}

/** Takes `user`, a session that closed, out of each set of sessions in `askers`; whether one still holds an open session. */
function stillAsked(askers: ReadonlySet<Set<object>>, user: object): boolean {
  let asked = false;
  for (const users of askers) {
    users.delete(user);
    asked ||= users.size > 0;
  }
  return asked;
}

/** Compile jobs run one at a time, in the order they were added. */
class CompileLine {
  readonly #waiting: CompileJob[] = [];
  #busy = false;

  add(job: CompileJob): void {
    this.#waiting.push(job);
    this.#next();
  }

  /** Takes `job` out of this line; false once it has started, or if it was never here. */
  take(job: CompileJob): boolean {
    const at = this.#waiting.indexOf(job);
    if (at < 0) return false;
    this.#waiting.splice(at, 1);
    return true;
  }

  #next(): void {
    if (this.#busy) return;
    const job = this.#waiting.shift();
    if (job === undefined) return;
    this.#busy = true;
    void job().finally(() => {
      this.#busy = false;
      this.#next();
    });
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
    return checkedOutcome(loadNativeBinding().CodeRuleDslRuntime.compile(sourceText, "json", capabilities.json), policyId);
  } catch (error) {
    return failedOutcome(error);
  }
}

/** `compileOutcome` on a thread of Node's worker pool; on the JS thread, as before, with an engine built before `compileAsync`. */
async function compileOutcomeOffThread(capabilities: CodeRuleCapabilities, policyId: string, sourceText: string): Promise<CompileOutcome> {
  try {
    const engine = loadNativeBinding().CodeRuleDslRuntime;
    if (engine.compileAsync === undefined) return compileOutcome(capabilities, policyId, sourceText);
    return checkedOutcome(await engine.compileAsync(sourceText, "json", capabilities.json), policyId);
  } catch (error) {
    return failedOutcome(error);
  }
}

function checkedOutcome(executable: NativeCompiledCodeRuleDsl, policyId: string): CompileOutcome {
  const canonical = JSON.parse(executable.canonicalJson()) as { id?: unknown };
  return canonical.id === policyId
    ? { ok: true, executable, crossFile: executable.readsOtherFiles() }
    : { ok: false, reason: `Compiled policy ID does not match rule document: ${policyId}` };
}

function failedOutcome(error: unknown): CompileOutcome {
  return { ok: false, reason: error instanceof Error ? error.message : String(error) };
}

const compiledRules = new CompiledRuleCache();

/** How many of a session's newest rule checks `checkoutsEditedBy` reads for the checkouts they ran in. */
const EDITED_AUDITS_READ = 500;
/** How many repositories `checkoutsEditedBy` names at most. */
const EDITED_REPOSITORIES_READ = 8;

export interface CompiledCodeRuleDslEntry {
  readonly id: string;
  readonly executable: NativeCompiledCodeRuleDsl;
  /** Where the code rule runs in the checkout, at which mode, and where suppression comments are ignored; passed to the native workspace. */
  readonly placement: CodeRulePlacement;
  /** Whether its finding in one file can depend on another file's text, as the engine routes its clauses (`readsOtherFiles`). */
  readonly crossFile: boolean;
  /** The rule document's own title, which the person's line names the rule by (`steerTitle`); absent when it has none. */
  readonly title?: string | undefined;
}

/** Immutable provenance and executables used to construct one native evaluation workspace. */
export interface CompiledCodeRuleDslSnapshot {
  readonly version: string;
  readonly codeRules: readonly CompiledCodeRuleDslEntry[];
  readonly provenance: readonly AuditRuleProvenance[];
}

export interface CodeRuleDslCatalogStatus {
  /** Code rules compiled and placed somewhere in the checkout. */
  readonly ruleCount: number;
  /** Digest of the compiled set: capabilities, and each code rule's id, source, source digest and placement. */
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

/** A code rule a catalog places, as its executable is keyed: its id, source and option values. */
export interface PlannedRule {
  readonly policyId: string;
  readonly rule: EffectiveRule;
  readonly decisions: RulePlacementDecision[];
  readonly placement: CodeRulePlacement;
  readonly sourceText: string;
  readonly sourceDigest: string;
  readonly config: string;
}

/** One load's code rules in id order: each one to compile, or left out with the reason. */
export type PlanEntry = { readonly compile: PlannedRule } | { readonly skip: QuarantinedRule };

/** What a catalog builds from one load: the configuration, and every code rule placed somewhere, in id order. */
export interface CatalogPlan {
  readonly ruleSet: CheckoutRuleSet | undefined;
  readonly quarantined: readonly QuarantinedRule[];
  readonly entries: readonly PlanEntry[];
}

/** A plan with the outcome of each rule it compiles, in its order, at the source signature it was loaded at. */
export interface CompiledPlan {
  readonly plan: CatalogPlan;
  readonly outcomes: readonly (CompileOutcome | undefined)[];
  readonly sourceSignature: string;
}

function catalogPlan(loaded: LoadedCheckoutRules): CatalogPlan {
  const ruleSet = loaded.ruleSet;
  const placed = ruleSet === undefined ? new Map<string, PlacedRule>() : placedRules(ruleSet, loaded.folders);
  const entries = [...placed.keys()].sort(compareDeterministicStrings)
    .flatMap(policyId => planEntry(policyId, placed.get(policyId)!, loaded.folders) ?? []);
  return { ruleSet, quarantined: loaded.quarantined, entries };
}

/** The rule `policyId` to compile, or left out with why; undefined where it is placed nowhere. */
function planEntry(policyId: string, { rule, decisions }: PlacedRule, folders: readonly string[]): PlanEntry | undefined {
  const placement = placementOf(decisions, folders);
  if (placement.length === 0) return undefined;
  const skipped = refusedBeforeCompile(policyId, rule);
  if (skipped !== undefined) return { skip: skipped };
  const source = ruleSourceText(rule.document.rule);
  if (source === undefined) throw new Error(`Policy ${policyId} is not JSON representable`);
  return { compile: { policyId, rule, decisions, placement, sourceText: source.text, sourceDigest: source.digest, config: stableJsonText(rule.config) ?? "" } };
}

/**
 * One checkout's code rules, compiled into resident executables and placed per folder. `refresh()` reloads only
 * when the source signature changes; an unchanged rule is never recompiled.
 */
export class CheckoutCodeRuleCatalog implements CodeRuleDslCatalog {
  readonly #source: CheckoutRuleSource;
  readonly #capabilities: CodeRuleCapabilities;
  #state: CatalogState;

  /** With `compiled` (from `built`), its rules as compiled there; without it, each rule no live catalog holds compiles here. */
  constructor(source: CheckoutRuleSource, capabilities: CodeRuleCapabilities = loadCodeRuleCapabilities(), compiled?: CompiledPlan) {
    this.#source = source;
    this.#capabilities = capabilities;
    if (compiled !== undefined) {
      this.#state = this.#stateOf(compiled);
      return;
    }
    // The signature is taken before the load, so a change made meanwhile shows at the next refresh.
    const sourceSignature = source.signature();
    this.#state = this.#build(source.load(), sourceSignature);
  }

  /**
   * `source`'s catalog as the constructor builds it, with each rule no live catalog holds compiled off the JS thread
   * (`CompiledRuleCache.compileOffThread`), so the daemon answers other sessions while they compile. `aheadFor`: the
   * open sessions it is built for before any tool call asks for it; one that joins the build is added to it
   * (`joinedBuild`), so each rule still waiting is built while one of them is open.
   */
  static async built(source: CheckoutRuleSource, capabilities: CodeRuleCapabilities, aheadFor?: Set<object>): Promise<CheckoutCodeRuleCatalog> {
    const sourceSignature = source.signature();
    const plan = catalogPlan(source.load());
    const outcomes = await Promise.all(plan.entries.map(async entry =>
      "compile" in entry ? await compiledRules.compileOffThread(capabilities, entry.compile, aheadFor) : undefined));
    return new CheckoutCodeRuleCatalog(source, capabilities, { plan, outcomes, sourceSignature });
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
    const plan = catalogPlan(loaded);
    const outcomes = plan.entries.map(entry => ("compile" in entry
      ? compiledRules.compile(this.#capabilities, entry.compile.policyId, entry.compile.sourceText, entry.compile.sourceDigest, entry.compile.config)
      : undefined));
    return this.#stateOf({ plan, outcomes, sourceSignature });
  }

  /** The catalog of a plan whose rules have compiled: each placed rule that compiled, and each left out with why. */
  #stateOf({ plan, outcomes, sourceSignature }: CompiledPlan): CatalogState {
    const quarantined = [...plan.quarantined];
    const built: BuiltRule[] = [];
    plan.entries.forEach((entry, index) => {
      const compiled = "skip" in entry ? { quarantined: entry.skip } : builtRule(entry.compile, outcomes[index]!, plan.ruleSet!.checkout);
      if ("quarantined" in compiled) quarantined.push(compiled.quarantined);
      else built.push(compiled);
    });
    return catalogState({ capabilitiesDigest: this.#capabilities.digest, label: this.#source.label, sourceSignature }, built, quarantined);
  }
}

/**
 * The build of `entry`'s catalog an ask can wait for: the one under way, unless it was asked ahead and this ask is a
 * call's, which builds its own so the rules it needs move to the line of calls (`CompiledRuleCache.compileOffThread`).
 * A session asking ahead (`aheadFor`) is added to those an ahead build and each rule it waits for are for.
 */
function joinedBuild(entry: CheckoutEntry, aheadFor: object | undefined): Promise<void> | undefined {
  const running = entry.building;
  if (running === undefined) return undefined;
  if (aheadFor === undefined) return running.users === undefined ? running.done : undefined;
  running.users?.add(aheadFor);
  return running.done;
}

/**
 * Keeps `built`, built for `users` ahead of any call (undefined: for a call), as `entry`'s catalog, unless each of them
 * closed meanwhile. Another ask may have built it meanwhile: the first one built stays, the one every lane of the
 * checkout holds, and a call's build keeps it.
 */
function keptBuilt(entry: CheckoutEntry, built: CheckoutCodeRuleCatalog, users: ReadonlySet<object> | undefined): void {
  if (entry.catalog !== undefined) {
    if (users === undefined) heldFor(entry, undefined);
    return;
  }
  if (users?.size === 0) return;
  entry.catalog = built;
  entry.heldAheadFor = users === undefined ? undefined : new Set(users);
}

/** Notes an ask of `entry`'s built catalog: a session asking ahead (`aheadFor`) holds it with the others, any other ask keeps it. */
function heldFor(entry: CheckoutEntry, aheadFor: object | undefined): void {
  if (aheadFor === undefined) entry.heldAheadFor = undefined;
  else entry.heldAheadFor?.add(aheadFor);
}

/**
 * Lets go of `user`, a session that closed, in `entry`: an ahead build under way is no longer for it, and a catalog
 * built ahead that only it held and nobody read is dropped, so the engine frees the rules no other catalog holds.
 */
function releasedFor(entry: CheckoutEntry, user: object): void {
  entry.building?.users?.delete(user);
  if (entry.heldAheadFor?.delete(user) !== true || entry.heldAheadFor.size > 0) return;
  entry.catalog = undefined;
  entry.heldAheadFor = undefined;
}

/** A planned rule with its compile's outcome: its entry, provenance and version line, or why it is quarantined. */
function builtRule(planned: PlannedRule, outcome: CompileOutcome, checkout: CheckoutContext): BuiltRule | { readonly quarantined: QuarantinedRule } {
  const { policyId: id, rule, decisions, placement, sourceDigest } = planned;
  if (!outcome.ok) return { quarantined: refusedRule(id, rule, outcome.reason) };
  return {
    entry: { id, executable: outcome.executable, placement, crossFile: outcome.crossFile, title: rule.document.rule.title },
    provenance: {
      ruleId: id, revision: rule.revision, payloadHash: rule.payloadHash, source: { ...rule.source },
      repositoryKey: checkout.repositoryKey, checkoutPath: checkout.checkoutPath, placement: decisions,
    },
    versioned: { id, sourceDigest, placement, source: rule.source },
  };
}

/** A rule the catalog compiled: its entry, its provenance, and its line of the catalog's version. */
interface BuiltRule {
  readonly entry: CompiledCodeRuleDslEntry;
  readonly provenance: RepositoryAuditRuleProvenance;
  readonly versioned: unknown;
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
  built: readonly BuiltRule[],
  quarantined: readonly QuarantinedRule[],
): CatalogState {
  for (const entry of quarantined) if (entry.code === undefined) reportQuarantinedRule(catalog.label, entry);
  const policies = built.map(rule => rule.versioned);
  const canonical = stableJsonText({ capabilitiesDigest: catalog.capabilitiesDigest, format: "rerail.policy-dsl-catalog/v3", policies });
  if (canonical === undefined) throw new Error("Policy catalog version is not JSON representable");
  const codeRules = built.map(rule => rule.entry);
  return {
    status: { ruleCount: codeRules.length, version: digest(canonical), sourceSignature: catalog.sourceSignature },
    snapshot: { version: digest(canonical), codeRules, provenance: built.map(rule => rule.provenance) },
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
  /**
   * The build of its catalog off the thread under way (`prepare`), and the sessions it is built for ahead of any call
   * that have not closed; undefined for a call's build.
   */
  building?: { readonly users: Set<object> | undefined; readonly done: Promise<void> } | undefined;
  /** The open sessions its catalog was built ahead for, while nothing else asked for it; undefined once something did. */
  heldAheadFor?: Set<object> | undefined;
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
  #closed = false;

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
    entry.catalog ??= new CheckoutCodeRuleCatalog(this.#sourceOf(entry), this.#capabilities);
    heldFor(entry, undefined);
    return entry.catalog;
  }

  /** Whether `root`'s catalog is built and held here, the one `catalog` gives without compiling. */
  holdsCatalog(root: string): boolean {
    return this.#entries.get(root)?.catalog !== undefined;
  }

  /**
   * Reads `root`'s catalog again (`CheckoutCodeRuleCatalog.refresh`) when it is built. One not built yet is left to
   * `prepare` or `catalog`, which read the rules as they are then, so no rule compiles here for it.
   */
  refreshCatalog(root: string): void {
    this.#entries.get(root)?.catalog?.refresh();
  }

  /**
   * Builds `root`'s catalog, the one `catalog` then gives, with each rule no live catalog holds compiled off the JS
   * thread (`CheckoutCodeRuleCatalog.built`); settles once it is built, at once when it already was. `aheadFor`: the
   * session it is built for before any tool call asks for it, so a call's own rules never wait behind it; once that
   * session and every other it was built ahead for closed (`releaseAhead`), a catalog nothing else asked for is dropped.
   */
  async prepare(root: string, aheadFor?: object): Promise<void> {
    if (this.#closed) return;
    const entry = this.#entry(root);
    if (entry.catalog === undefined) await (joinedBuild(entry, aheadFor) ?? this.#build(entry, aheadFor));
    else heldFor(entry, aheadFor);
  }

  /**
   * Lets go of what `user`, a session that closed, asked for ahead of any call (`prepare`): each catalog built for it
   * alone that nothing else asked for is dropped, one building for it alone is not kept, and its rules still waiting
   * on the line of rules built ahead never compile.
   */
  releaseAhead(user: object): void {
    for (const entry of this.#entries.values()) releasedFor(entry, user);
    compiledRules.releaseAhead(user);
  }

  /** `entry`'s catalog built off the thread; while it builds, an ask of the same kind joins it (`joinedBuild`). */
  async #build(entry: CheckoutEntry, aheadFor: object | undefined): Promise<void> {
    const users = aheadFor === undefined ? undefined : new Set([aheadFor]);
    const done = CheckoutCodeRuleCatalog.built(this.#sourceOf(entry), this.#capabilities, users).then(built => keptBuilt(entry, built, users));
    const building = { users, done };
    entry.building = building;
    try {
      await done;
    } finally {
      if (entry.building === building) entry.building = undefined;
    }
  }

  /**
   * The checkouts a session's rule checks ran in, newest first, one per repository and at most `limit`: where it edited
   * before it closed or its daemon stopped, and so whose rules its next edits most likely need. A checkout's other
   * worktrees share its repository's rules, so one checkout per repository builds them.
   */
  checkoutsEditedBy(sessionId: string, limit = EDITED_REPOSITORIES_READ): string[] {
    const roots = this.database.select({ root: audits.workspaceRoot }).from(audits)
      .where(eq(audits.sessionId, sessionId)).orderBy(desc(audits.startedAt)).limit(EDITED_AUDITS_READ).all()
      .map(row => row.root);
    if (roots.length === 0) return [];
    const repositories = new Map<string, string>();
    const keys = new Map(this.database.select({ root: checkouts.checkoutPath, key: checkouts.repositoryKey }).from(checkouts)
      .where(inArray(checkouts.checkoutPath, [...new Set(roots)])).all().map(row => [row.root, row.key]));
    for (const root of roots) {
      const key = keys.get(root);
      if (key !== undefined && !repositories.has(key) && repositories.size < limit) repositories.set(key, root);
    }
    return [...repositories.values()];
  }

  #sourceOf(entry: CheckoutEntry): CheckoutRuleSource {
    return { label: this.databasePath, signature: () => this.#signature(entry), load: () => this.#load(entry) };
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
    this.#closed = true;
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
