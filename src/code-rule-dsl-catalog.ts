import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { resolve } from "node:path";
import { loadNativeBinding, type NativeCompiledCodeRuleDsl } from "./native.js";
import { bundledCodeRuleDslDirectory } from "./code-rule-compile.js";
import { reportQuarantinedRule, type QuarantinedRule } from "./rule-quarantine.js";
import type { AuditRuleProvenance } from "./audit.js";
import { SqliteRuleStore } from "./rule-store.js";
import type { ScopedRuleCatalog } from "./scoped-rule-catalog.js";
import { ruleDocumentDigest, stableJsonText } from "./stable-json.js";

interface SourcedCodeRule {
  policyId: string;
  revision: number;
  sourceDigest: string;
  sourceText: string;
}

interface CatalogSource {
  catalogDigest: string;
  codeRules: SourcedCodeRule[];
  provenance: AuditRuleProvenance[];
  quarantined: QuarantinedRule[];
}

export interface CodeRuleDslCatalogStatus {
  capabilitiesPath: string;
  /** The authoritative catalog the enabled `file` rows are compiled from. */
  databasePath: string;
  ruleCount: number;
  /** Digest of the compiled set: capabilities plus every code rule's id and source digest. */
  version: string;
  /** Source identity captured before loading; never advanced by presentation reads or failed compilation. */
  sourceSignature: string;
}

export interface CompiledCodeRuleDslEntry {
  id: string;
  executable: NativeCompiledCodeRuleDsl;
}

/** Immutable provenance and executables used to construct one native evaluation workspace. */
export interface CompiledCodeRuleDslSnapshot {
  version: string;
  codeRules: readonly CompiledCodeRuleDslEntry[];
  ruleRevisions: readonly { ruleId: string; revision: number }[];
  provenance: readonly AuditRuleProvenance[];
}

export interface BootstrapCodeRuleDslCatalogOptions {
  capabilitiesPath?: string;
  /** Catalog path shown in diagnostics when `store` is supplied. */
  databasePath: string;
  /** Effective rules for one repository; without it the `rules` table at `databasePath` is read. */
  store?: SqliteRuleStore | ScopedRuleCatalog;
  onRefresh?: () => void;
}

function digest(value: string): string {
  return `sha256:${createHash("sha256").update(value).digest("hex")}`;
}

export { bundledCodeRuleDslDirectory };

/** Code-point order, matching the stable-JSON key order the digest uses; `localeCompare` would vary by locale. */
function compareIds(left: string, right: string): number {
  if (left === right) return 0;
  return left < right ? -1 : 1;
}

function catalogSource(
  codeRules: SourcedCodeRule[],
  provenance: AuditRuleProvenance[],
  quarantined: QuarantinedRule[],
  capabilitiesJson: string,
): CatalogSource {
  const canonical = stableJsonText({
    capabilitiesDigest: digest(capabilitiesJson),
    format: "rerail.policy-dsl-catalog/v2",
    policies: codeRules.map(({ policyId, sourceDigest }, index) => ({
      policyId,
      sourceDigest,
      provenance: provenance[index],
    })),
  });
  if (canonical === undefined) throw new Error("Policy catalog version is not JSON representable");
  return { catalogDigest: digest(canonical), codeRules, provenance, quarantined };
}

function loadCatalogSource(store: SqliteRuleStore | ScopedRuleCatalog, capabilitiesJson: string): CatalogSource {
  const storedCodeRules = store.list({ kind: "file", enabled: true }).flatMap(stored =>
    stored.kind === "file" ? [stored] : []);
  const seen = new Set<string>();
  const orderedCodeRules = storedCodeRules.toSorted((left, right) => compareIds(left.rule.id, right.rule.id));
  const sourced = orderedCodeRules.map(stored => {
    const codeRule = stored.rule;
    if (seen.has(codeRule.id)) throw new Error(`Duplicate executable policy ID: ${codeRule.id}`);
    seen.add(codeRule.id);
    const sourceText = stableJsonText(codeRule);
    if (sourceText === undefined) throw new Error(`Policy ${codeRule.id} is not JSON representable`);
    return { policyId: codeRule.id, revision: stored.revision, sourceDigest: digest(sourceText), sourceText };
  });
  const provenance = orderedCodeRules.map(stored => {
    let applicability: "global" | "project";
    if ("applicability" in stored) applicability = stored.applicability;
    else applicability = stored.scope === "global" ? "global" : "project";
    return {
      ruleId: stored.rule.id,
      revision: stored.revision,
      payloadHash: "payloadHash" in stored
        ? stored.payloadHash
        : ruleDocumentDigest({ kind: "file", rule: stored.rule }),
      applicability,
      ...("origin" in stored && stored.origin.kind === "project" ? { projectId: stored.origin.projectId } : {}),
      origin: "catalogOrigin" in stored ? stored.catalogOrigin : "user" as const,
    };
  });
  return catalogSource(sourced, provenance, store.quarantined().filter(entry => entry.kind === "file"), capabilitiesJson);
}

function compileCodeRule(codeRule: SourcedCodeRule, capabilitiesJson: string): NativeCompiledCodeRuleDsl {
  const executable = loadNativeBinding().CodeRuleDslRuntime.compile(codeRule.sourceText, "json", capabilitiesJson);
  const canonical = JSON.parse(executable.canonicalJson()) as { id?: unknown };
  if (canonical.id !== codeRule.policyId) {
    throw new Error(`Compiled policy ID does not match rule document: ${codeRule.policyId}`);
  }
  return executable;
}

export interface CodeRuleDslCatalog {
  readonly status: CodeRuleDslCatalogStatus;
  codeRule(id: string): NativeCompiledCodeRuleDsl;
  provenance(): readonly AuditRuleProvenance[];
  codeRules(): readonly CompiledCodeRuleDslEntry[];
  /** Revisions of the exact enabled rules represented by the current catalog digest. */
  ruleRevisions(): readonly { ruleId: string; revision: number }[];
  /** The immutable compiled catalog state from which one native workspace is built. */
  snapshot(): CompiledCodeRuleDslSnapshot;
  /** Code-rule rows the last successful load skipped because they fail validation; recomputed only when the source changes. */
  quarantined(): readonly QuarantinedRule[];
  /**
   * Re-read enabled code rules only when their source identity changes,
   * recompiling only changed bodies. Returns whether the compiled set differs.
   */
  refresh(): boolean;
  close(): void;
}

/**
 * The enabled `file` rows of the project database, compiled into resident
 * Rust executables. The database is the only source: a row toggled, edited,
 * or deleted by the daemon, the UI, or raw SQL is picked up by the next
 * `refresh()`. Compiled executables are keyed by source digest so an
 * unchanged code rule is never recompiled.
 */
class LiveRuleCatalog implements CodeRuleDslCatalog {
  status: CodeRuleDslCatalogStatus;
  readonly #store: SqliteRuleStore | ScopedRuleCatalog;
  readonly #capabilitiesJson: string;
  #compiled = new Map<string, { sourceDigest: string; executable: NativeCompiledCodeRuleDsl }>();
  #provenance: readonly AuditRuleProvenance[] = [];
  #ruleRevisions: { ruleId: string; revision: number }[] = [];
  #quarantined: readonly QuarantinedRule[] = [];
  #closed = false;

  constructor(store: SqliteRuleStore | ScopedRuleCatalog, capabilitiesPath: string, capabilitiesJson: string, readonly onRefresh?: () => void) {
    this.#store = store;
    this.#capabilitiesJson = capabilitiesJson;
    const sourceSignature = "ruleSignature" in store ? store.ruleSignature("file") : String(store.dataVersion());
    const source = loadCatalogSource(store, capabilitiesJson);
    const applied = this.#apply(source);
    this.status = { capabilitiesPath, databasePath: store.path, ruleCount: applied.codeRules.length, version: applied.catalogDigest, sourceSignature };
  }

  codeRule(id: string): NativeCompiledCodeRuleDsl {
    this.#assertOpen();
    const entry = this.#compiled.get(id);
    if (entry === undefined) throw new Error(`Unknown policy DSL ID: ${id}`);
    return entry.executable;
  }

  codeRules(): readonly CompiledCodeRuleDslEntry[] {
    this.#assertOpen();
    return [...this.#compiled].map(([id, { executable }]) => ({ id, executable }));
  }

  provenance(): readonly AuditRuleProvenance[] {
    this.#assertOpen();
    return this.#provenance;
  }

  ruleRevisions(): readonly { ruleId: string; revision: number }[] {
    this.#assertOpen();
    return [...this.#ruleRevisions];
  }

  quarantined(): readonly QuarantinedRule[] {
    this.#assertOpen();
    return this.#quarantined;
  }

  snapshot(): CompiledCodeRuleDslSnapshot {
    this.#assertOpen();
    return {
      version: this.status.version,
      codeRules: this.codeRules(),
      ruleRevisions: this.ruleRevisions(),
      provenance: this.provenance(),
    };
  }

  refresh(): boolean {
    this.#assertOpen();
    const sourceSignature = "ruleSignature" in this.#store ? this.#store.ruleSignature("file") : String(this.#store.dataVersion());
    if (sourceSignature === this.status.sourceSignature) return false;
    const source = loadCatalogSource(this.#store, this.#capabilitiesJson);
    const applied = this.#apply(source);
    const changed = applied.catalogDigest !== this.status.version;
    this.status = { ...this.status, ruleCount: applied.codeRules.length, version: applied.catalogDigest, sourceSignature };
    this.onRefresh?.();
    return changed;
  }

  close(): void {
    if (this.#closed) return;
    this.#closed = true;
    this.#compiled.clear();
    this.#store.close();
  }

  #assertOpen(): void {
    if (this.#closed) throw new Error("Policy DSL catalog is closed");
  }

  /** Compile each valid stored code rule independently so one engine-invalid row is quarantined, not fatal. */
  #apply(source: CatalogSource): CatalogSource {
    const next = new Map<string, { sourceDigest: string; executable: NativeCompiledCodeRuleDsl }>();
    const codeRules: SourcedCodeRule[] = [];
    const provenance: AuditRuleProvenance[] = [];
    const quarantined = [...source.quarantined];
    for (const [index, codeRule] of source.codeRules.entries()) {
      const previous = this.#compiled.get(codeRule.policyId);
      const reusable = previous?.sourceDigest === codeRule.sourceDigest ? previous.executable : undefined;
      try {
        next.set(codeRule.policyId, { sourceDigest: codeRule.sourceDigest, executable: reusable ?? compileCodeRule(codeRule, this.#capabilitiesJson) });
        codeRules.push(codeRule);
        provenance.push(source.provenance[index]!);
      } catch (error) {
        const origin = source.provenance[index]!;
        const quarantinedCodeRule = {
          ruleId: codeRule.policyId,
          kind: "file" as const,
          owner: origin.applicability === "global" ? "global" : `project:${origin.projectId ?? "unknown"}`,
          reason: error instanceof Error ? error.message : String(error),
        };
        quarantined.push(quarantinedCodeRule);
        reportQuarantinedRule(this.#store.path, quarantinedCodeRule);
      }
    }
    const applied = catalogSource(codeRules, provenance, quarantined, this.#capabilitiesJson);
    this.#ruleRevisions = applied.codeRules.map(({ policyId, revision }) => ({ ruleId: policyId, revision }));
    this.#compiled = next;
    this.#provenance = applied.provenance;
    this.#quarantined = applied.quarantined;
    return applied;
  }
}

export function bootstrapCodeRuleDslCatalog(options: BootstrapCodeRuleDslCatalogOptions): CodeRuleDslCatalog {
  const capabilitiesPath = resolve(options.capabilitiesPath ?? resolve(bundledCodeRuleDslDirectory(), "capabilities.json"));
  const capabilitiesJson = readFileSync(capabilitiesPath, "utf8");
  const store = options.store ?? new SqliteRuleStore(resolve(options.databasePath));
  try {
    return new LiveRuleCatalog(store, capabilitiesPath, capabilitiesJson, options.onRefresh);
  } catch (error) {
    store.close();
    throw error;
  }
}
