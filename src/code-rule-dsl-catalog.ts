/**
 * Compiled file rules per checkout. A checkout's catalog compiles the file rules its repository's
 * configuration makes present and places each one per folder (on at a mode, or off), so one native workspace
 * evaluates every folder of the checkout. Executables are compiled once per process and shared by every catalog.
 */
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { and, eq, inArray, or } from "drizzle-orm";
import { loadBundledCatalog } from "./bundled-rules.js";
import { physicalPath, withinCheckout } from "./checkout-control.js";
import { CheckoutOwners, type CheckoutOwner } from "./checkout-owner.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "./database.js";
import { reportQuarantinedRule, type QuarantinedRule } from "./rule-quarantine.js";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "./native.js";
import type { RepositoryAuditRuleProvenance, AuditRuleProvenance, RulePlacementDecision } from "./audit.js";
import { bundledCodeRuleDslDirectory } from "./code-rule-compile.js";
import {
  folderContains,
  type BundledCatalog,
  type CheckoutContext,
  type CheckoutRuleSet,
  type EffectiveRule,
  type CodeRulePlacement,
  type CodeRulePlacementEntry,
  type RepositoryStatus,
} from "./repository-model.js";
import { repositoryIdentityLocation } from "./repository-identity.js";
import { parentCheckouts } from "./repository-registry.js";
import { checkoutStatus } from "./repository-start.js";
import { loadCheckoutRuleSet, ruleConfigurationSignature } from "./rule-configuration.js";
import { LIBRARY_OWNER, quarantinedLibraryRules } from "./rule-library.js";
import type { ConversationRule } from "./rule-schema.js";
import type { ConversationRuleLoader } from "./rule-sources.js";
import { checkouts, ruleAttachments, ruleSettings } from "./schema.js";
import { compareDeterministicStrings, stableJsonText } from "./stable-json.js";

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

type CompileOutcome = { readonly ok: true; readonly executable: NativeCompiledCodeRuleDsl } | { readonly ok: false; readonly reason: string };

/** Compiled executables keyed by capabilities and source, shared by every checkout of the process. */
class CompiledRuleCache {
  readonly #outcomes = new Map<string, CompileOutcome>();

  compile(capabilities: CodeRuleCapabilities, policyId: string, sourceText: string, sourceDigest: string): CompileOutcome {
    const key = `${capabilities.digest}\0${sourceDigest}`;
    let outcome = this.#outcomes.get(key);
    if (outcome === undefined) {
      try {
        const executable = loadNativeBinding().CodeRuleDslRuntime.compile(sourceText, "json", capabilities.json);
        const canonical = JSON.parse(executable.canonicalJson()) as { id?: unknown };
        outcome = canonical.id === policyId
          ? { ok: true, executable }
          : { ok: false, reason: `Compiled policy ID does not match rule document: ${policyId}` };
      } catch (error) {
        outcome = { ok: false, reason: error instanceof Error ? error.message : String(error) };
      }
      this.#outcomes.set(key, outcome);
    }
    return outcome;
  }
}

const compiledRules = new CompiledRuleCache();

export interface CompiledCodeRuleDslEntry {
  readonly id: string;
  readonly executable: NativeCompiledCodeRuleDsl;
  /** Where the code rule runs in the checkout, and at which mode; passed to the native workspace. */
  readonly placement: CodeRulePlacement;
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
  /** Every folder where presence, `on` or `mode` can change: attachment and setting folders, and `""`. */
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

/** Each file rule present in some folder, with how it runs in every folder it is present in. */
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
      entry.decisions.push({ folder, on: rule.on, mode: rule.mode, onFrom: rule.decidedBy.on, modeFrom: rule.decidedBy.mode });
    }
  }
  return placed;
}

/**
 * The native placement: per folder the mode (or null, off; a folder the rule is absent from is off), keeping only
 * folders that differ from the deepest kept folder containing them. A rule off everywhere has no entries.
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
    if (parent === undefined ? mode !== null : parent.mode !== mode) kept.push({ folder, mode });
  }
  return kept;
}

interface CatalogState {
  readonly status: CodeRuleDslCatalogStatus;
  readonly snapshot: CompiledCodeRuleDslSnapshot;
  readonly quarantined: readonly QuarantinedRule[];
}

/**
 * One checkout's file rules, compiled into resident executables and placed per folder. `refresh()` reloads only
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
    const codeRules: CompiledCodeRuleDslEntry[] = [];
    const provenance: RepositoryAuditRuleProvenance[] = [];
    const versioned: unknown[] = [];
    const ruleSet = loaded.ruleSet;
    const placed = ruleSet === undefined ? new Map<string, PlacedRule>() : placedRules(ruleSet, loaded.folders);
    for (const id of [...placed.keys()].sort(compareDeterministicStrings)) {
      const { rule, decisions } = placed.get(id)!;
      const placement = placementOf(decisions, loaded.folders);
      if (placement.length === 0) continue;
      const sourceText = stableJsonText(rule.document.rule);
      if (sourceText === undefined) throw new Error(`Policy ${id} is not JSON representable`);
      const sourceDigest = digest(sourceText);
      const outcome = compiledRules.compile(this.#capabilities, id, sourceText, sourceDigest);
      if (!outcome.ok) {
        quarantined.push({ ruleId: id, kind: "file", owner: rule.source.kind === "library_rule" ? LIBRARY_OWNER : RERAIL_OWNER, reason: outcome.reason });
        continue;
      }
      codeRules.push({ id, executable: outcome.executable, placement });
      provenance.push({
        ruleId: id,
        revision: rule.revision,
        payloadHash: rule.payloadHash,
        source: { ...rule.source },
        repositoryKey: ruleSet!.checkout.repositoryKey,
        checkoutPath: ruleSet!.checkout.checkoutPath,
        placement: decisions,
      });
      versioned.push({ id, sourceDigest, placement, source: rule.source });
    }
    for (const entry of quarantined) reportQuarantinedRule(this.#source.label, entry);
    const canonical = stableJsonText({ capabilitiesDigest: this.#capabilities.digest, format: "rerail.policy-dsl-catalog/v3", policies: versioned });
    if (canonical === undefined) throw new Error("Policy catalog version is not JSON representable");
    return {
      status: { ruleCount: codeRules.length, version: digest(canonical), sourceSignature },
      snapshot: { version: digest(canonical), codeRules, provenance },
      quarantined,
    };
  }
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
  location: { readonly git: boolean; readonly main: boolean } | undefined;
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
  readonly bundle: BundledCatalog;
  readonly #capabilities: CodeRuleCapabilities;
  readonly #entries = new Map<string, CheckoutEntry>();

  constructor(databasePath: string = defaultRerailDatabasePath(), options: { capabilities?: CodeRuleCapabilities; bundle?: BundledCatalog } = {}) {
    this.databasePath = resolve(databasePath);
    this.database = openRerailDatabase(this.databasePath);
    this.bundle = options.bundle ?? loadBundledCatalog();
    this.#capabilities = options.capabilities ?? loadCodeRuleCapabilities();
  }

  /** The checkout at `root` (a physical checkout top level) as last read; refreshed with `refresh`. */
  checkout(root: string): CheckoutContext | undefined {
    return this.#entry(root).checkout;
  }

  eligibility(root: string): CheckoutEligibility {
    return this.#entry(root).eligibility;
  }

  /** The checkout and its recorded parent checkouts, whose settings apply in it. */
  ownerCheckoutPaths(root: string): readonly string[] {
    const entry = this.#entry(root);
    return entry.checkout === undefined ? [] : [entry.checkout.checkoutPath, ...entry.parents];
  }

  /** Re-read `root`'s record, start state and set-up; returns its eligibility. */
  refresh(root: string): CheckoutEligibility {
    const entry = this.#entry(root, false);
    this.#read(entry);
    return entry.eligibility;
  }

  /** The recorded checkouts strictly inside `root` (physical paths), at any depth. */
  recordedCheckoutsWithin(root: string): string[] {
    return this.database.select({ checkoutPath: checkouts.checkoutPath }).from(checkouts).all()
      .map(row => row.checkoutPath)
      .filter(path => path !== root && withinCheckout(root, path));
  }

  /** The compiled file rules of `root`; its `refresh()` re-reads eligibility and configuration. */
  catalog(root: string): CheckoutCodeRuleCatalog {
    const entry = this.#entry(root);
    entry.catalog ??= new CheckoutCodeRuleCatalog({
      label: this.databasePath,
      signature: () => this.#signature(entry),
      load: () => this.#load(entry),
    }, this.#capabilities);
    return entry.catalog;
  }

  /** The conversation rules of the session folder `folder` (relative to `root`), as they change. */
  conversationRules(root: string, folder: string, load?: ConversationRuleLoader): ConversationRuleSource {
    const entry = this.#entry(root);
    return {
      signature: () => this.#signature(entry),
      rules: () => {
        const ruleSet = this.#ruleSet(entry);
        if (ruleSet === undefined || entry.eligibility !== "ok") return [];
        const configured = ruleSet.conversationRules(folder);
        const enabled = configured.filter(rule => rule.on).map(rule => rule.document.rule as ConversationRule);
        return load === undefined ? enabled : load({ enabled, ids: new Set(configured.map(rule => rule.document.rule.id)) });
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
      if (read) this.#read(entry);
    }
    return entry;
  }

  /** Only a recorded checkout is eligible; its row names its repository, so Git is asked once per checkout. */
  #read(entry: CheckoutEntry): void {
    const row = this.database.select({ repositoryKey: checkouts.repositoryKey, cloneId: checkouts.cloneId })
      .from(checkouts).where(eq(checkouts.checkoutPath, entry.root)).get();
    if (row === undefined) {
      entry.checkout = undefined;
      entry.parents = [];
      entry.status = undefined;
      entry.eligibility = "unrecorded";
      return;
    }
    if (entry.location === undefined) {
      const { git, main } = repositoryIdentityLocation(entry.root);
      entry.location = { git, main };
    }
    const checkout: CheckoutContext = { checkoutPath: entry.root, repositoryKey: row.repositoryKey, cloneId: row.cloneId, ...entry.location };
    const status = checkoutStatus(this.database, checkout);
    entry.checkout = checkout;
    entry.parents = parentCheckouts(this.database, checkout).map(parent => parent.checkoutPath);
    entry.status = status;
    if (!status.started) entry.eligibility = "stopped";
    else entry.eligibility = status.setUp ? "ok" : "not-set-up";
  }

  /** `<eligibility>` while unrecorded, else `<eligibility>\0<configuration signature>`; see `configurationSignatureOf`. */
  #signature(entry: CheckoutEntry): string {
    this.#read(entry);
    if (entry.checkout === undefined) return entry.eligibility;
    const signature = ruleConfigurationSignature(this.database, this.bundle, entry.checkout.repositoryKey, this.ownerCheckoutPaths(entry.root));
    return `${entry.eligibility}\0${signature}`;
  }

  #ruleSet(entry: CheckoutEntry): CheckoutRuleSet | undefined {
    if (entry.checkout === undefined || entry.status === undefined) return undefined;
    return loadCheckoutRuleSet(this.database, this.bundle, entry.checkout, entry.status, entry.parents);
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
    const quarantined = quarantinedLibraryRules(this.database).filter(rule => rule.kind === "file" && missingLibrary.has(rule.ruleId));
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
export function openCheckoutConversationRules(databasePath: string, cwd: string, load?: ConversationRuleLoader): ConversationRuleSource {
  const catalogs = new CheckoutCatalogs(databasePath);
  const start = startingCheckout(new CheckoutOwners(), cwd);
  const source = catalogs.conversationRules(start.checkoutRoot, start.path, load);
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
