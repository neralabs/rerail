/**
 * Internal testing: the headless `rerail scan` (no `--session`). It audits one checkout in this process, with no daemon
 * and no session, and reports a stable JSON document (`rerail.scan/1`, docs/headless-scan.md) a CI job or another tool
 * can read. Nothing is added to the Rerail database's session history: the audit is recorded in memory. (With the
 * home's configuration it opens that database as the daemon would, which applies this version's schema to it.)
 *
 * Which rules run:
 * - The Rerail home's configuration for the checkout, when Rerail is started and set up in it there (packs attached,
 *   approvals, settings), and neither `--pack` nor `--rules-ref` is given.
 * - Otherwise a throwaway database: the checkout's own `.rerail/rules/` (unapproved, so they run in Log), plus each
 *   `--pack` installed in the Rerail home. With `--rules-ref <ref>` the rules come from `.rerail/rules/` at that Git
 *   ref instead of the working tree, which the change under review cannot edit. Either way the approval policy is a
 *   session's: content no one approved in the Rerail home (`rerail rules approve`) runs in Log.
 */
import { createHash, randomUUID } from "node:crypto";
import { existsSync, mkdirSync, mkdtempSync, realpathSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { eq } from "drizzle-orm";
import type { AuditInput, AuditRecorder, AuditResult, AuditRuleProvenance } from "../engine/audit/audit.js";
import { CheckoutOwners } from "../engine/catalog/checkout-owner.js";
import { checkoutStatus, lookupCheckout, parentCheckouts, recordCheckout, standaloneRepository } from "../engine/catalog/repository-registry.js";
import type { RrPolicySource, RuleCatalog } from "../engine/catalog/repository-model.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { rulePackPackageOf } from "../engine/catalog/rule-packs.js";
import { resolveRrModes, setRuleSetting, setUpRepository } from "../engine/catalog/rule-configuration.js";
import { upsertLibraryRule } from "../engine/catalog/rule-library.js";
import type { RuleDocument } from "../engine/catalog/rule-store.js";
import { isWorktreePack, MAX_RULE_DOCUMENT_BYTES, rrPayloadHash, WORKTREE_PACK_ID, WORKTREE_RULES_FOLDER, worktreeRuleCatalog, worktreeScanCatalog } from "../engine/catalog/worktree-pack.js";
import { CheckoutCodeRuleRouter } from "../engine/code-rule/checkout-code-rule-router.js";
import { CheckoutCatalogs, startingCheckout } from "../engine/code-rule/code-rule-dsl-catalog.js";
import { loadNativeBinding, type NativeAnalysisScopeRun, type NativeGitRevisionSource, type NativeLoadedRrPolicies } from "../engine/native/native.js";
import { acquireDirectoryAnalysisGeneration, analysisScopeHome, scanContentStore } from "../engine/native/analysis-scope.js";
import { assertContentStoreOwner } from "../engine/native/main-instance.js";
import { SOURCE_LANGUAGES, SOURCE_NAMES } from "../engine/code-rule/code-rule-languages.js";
import { runGit } from "../engine/git.js";
import { parseJsonWithoutDuplicateKeys } from "../engine/json-duplicate-keys.js";
import { compareDeterministicStrings } from "../engine/stable-json.js";
import { defaultRerailDatabasePath, openRerailDatabase, openRerailDatabaseAsIs, type RerailDatabase } from "../engine/storage/database.js";
import { repositories } from "../engine/storage/schema.js";
import { tableColumns } from "../engine/storage/table-probe.js";
import { setRepositoryStarted, stateOf } from "../runtime/repository-start.js";

export const SCAN_SCHEMA = "rerail.scan/1";

export interface ScanPosition {
  /** 1-based. */
  line: number;
  /** 1-based, in characters as the engine counts them. */
  column: number;
}

export interface ScanRule {
  ruleId: string;
  payloadHash: string;
  mode: string;
  source: { kind: string; id: string; packId: string | null; ref?: string; commit?: string };
}

interface RefRuleSource {
  kind: "ref";
  ref: string;
  commit: string;
}

export interface ScanViolation {
  ruleId: string;
  payloadHash: string | null;
  mode: string;
  code: string;
  clauseId: string;
  path: string | null;
  /** Inclusive start. */
  start: ScanPosition | null;
  /** Exclusive end: the position just past the match. */
  end: ScanPosition | null;
  message: string;
  /** Stable across scans while the rule, path and matched text are unchanged (see docs/headless-scan.md). */
  fingerprint: string;
}

export interface ScanFailure {
  code: string;
  message: string;
  path: string | null;
  ruleId: string | null;
}

/** The headless scan's report: contract `rerail.scan/1`. */
export interface ScanReport {
  schema: typeof SCAN_SCHEMA;
  scanId: string;
  tool: { name: "rerail"; version: string };
  startedAt: number;
  finishedAt: number;
  repository: { key: string; displayName: string };
  checkout: { path: string; head: string | null; ref: string | null };
  source?: { revision: string; tree: string; commit: string | null };
  scope: "repository" | "git-diff";
  base: { ref: string; mergeBase: string } | null;
  paths: string[] | null;
  requestedPaths?: string[];
  skippedPaths?: string[];
  rulesSource: { kind: "home" } | { kind: "worktree" } | RefRuleSource | { kind: "rr" };
  status: "completed" | "failed";
  rules: ScanRule[];
  violations: ScanViolation[];
  failures: ScanFailure[];
  cacheWarnings?: string[];
  summary: { violations: number; failures: number };
}

export interface HeadlessScanOptions {
  repo: string;
  scope: "repository" | "git-diff";
  base?: string;
  rulesRef?: string;
  revision?: string;
  analysisScope?: string;
  packs: readonly string[];
  rr?: readonly string[];
  /**
   * With `scope: "repository"`, scan only these files (relative to the checkout) instead of the whole checkout; those
   * that do not exist there are left out. Undefined scans everything.
   */
  paths?: readonly string[];
  version: string;
  /** Defaults to `$RERAIL_HOME/rerail.sqlite`. */
  databasePath?: string;
  /**
   * Filled with the documents of the rules read from `rulesRef`, by payload hash: they exist nowhere else afterwards,
   * so a remote that receives the scan gets them from here.
   */
  ruleDocuments?: Map<string, RuleDocument>;
}

function git(cwd: string, args: readonly string[]): string {
  const result = runGit(cwd, args, { maxBuffer: 256 * 1024 * 1024 });
  if (result.error !== undefined) throw result.error;
  if (result.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${result.stderr.trim()}`);
  return result.stdout;
}

function gitMaybe(cwd: string, args: readonly string[]): string | null {
  const result = runGit(cwd, args);
  return result.status === 0 ? result.stdout.replace(/\n$/u, "") : null;
}

function nulList(text: string): string[] {
  return text.split("\0").filter(entry => entry.length > 0);
}

/**
 * Changed files against `against` (the working tree, staged changes included) plus untracked files, sorted. With
 * `--no-renames` a rename is its new path, and Git needs no blob it would fetch to detect one: lazy fetch is off.
 */
function diffPaths(root: string, against: string): string[] {
  const changed = nulList(git(root, ["diff", "--name-only", "--no-renames", "--relative", "--diff-filter=ACMRTUXB", "-z", against, "--"]));
  const untracked = nulList(git(root, ["ls-files", "--others", "--exclude-standard", "-z", "--"]));
  return [...new Set([...changed, ...untracked])].sort((left, right) => left.localeCompare(right));
}

/** A rule read from `--rules-ref`, and what loading the ref's rules left out (each with why), as a session would. */
interface RefRules {
  readonly provenance: RefRuleSource;
  readonly documents: RuleDocument[];
  readonly skipped: ScanFailure[];
  readonly rr?: { catalog: RuleCatalog; source: NativeGitRevisionSource };
}

/** Load the ref's policies through the same catalog rules used for a checkout. */
function rulesAtRef(root: string, ref: string, loadedRr: NativeLoadedRrPolicies): RefRules {
  const binding = loadNativeBinding();
  let source: NativeGitRevisionSource;
  try {
    source = new binding.GitRevisionSource(root, ref, "rules-ref");
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(`Cannot resolve --rules-ref ${ref} in ${root}. ${detail}`, { cause: error });
  }
  const commit = source.commit;
  if (!commit) throw new Error(`--rules-ref ${ref} resolved to tree ${source.tree}. This option requires a commit.`);
  const provenance: RefRuleSource = { kind: "ref", ref, commit };
  const failures: ScanFailure[] = [];
  const unreadable = (path: string, error: unknown) => {
    failures.push({
      code: "adapter-error", path: `${ref}:${path}`, ruleId: null,
      message: error instanceof Error ? error.message : String(error),
    });
  };
  let entries: NonNullable<ReturnType<NativeGitRevisionSource["readDirectory"]>>;
  try {
    entries = source.readDirectory(WORKTREE_RULES_FOLDER) ?? [];
  } catch (error) {
    unreadable(WORKTREE_RULES_FOLDER, error);
    return { provenance, documents: [], skipped: failures };
  }
  const files = entries.flatMap(({ name, nameBytes, mode }) => {
    if (name == null) {
      const bytes = nameBytes?.toString("latin1");
      if (nameBytes !== undefined && (bytes?.endsWith(".json") || bytes?.endsWith(".rr"))) {
        unreadable(`${WORKTREE_RULES_FOLDER}/[hex:${nameBytes.toString("hex")}]`, new Error("Git policy name has invalid UTF-8"));
      }
      return [];
    }
    return [{ name, path: `${WORKTREE_RULES_FOLDER}/${name}`, regular: mode === 0o100644 || mode === 0o100755, size: 0 }];
  }).sort((left, right) => compareDeterministicStrings(left.name, right.name));
  const jsonText = new Map<string, string>();
  const utf8 = new TextDecoder("utf-8", { fatal: true, ignoreBOM: true });
  const json = files.filter(file => file.name.endsWith(".json")).filter(file => {
    if (!file.regular) return true;
    try {
      const contents = source.readFile(file.path, MAX_RULE_DOCUMENT_BYTES);
      if (contents == null) file.size = MAX_RULE_DOCUMENT_BYTES + 1;
      else {
        try {
          jsonText.set(file.path, utf8.decode(contents));
        } catch (error) {
          throw new Error("Git JSON policy has invalid UTF-8", { cause: error });
        }
      }
      return true;
    } catch (error) {
      unreadable(file.path, error);
      return false;
    }
  });
  const rr = files.filter(file => file.name.endsWith(".rr"));
  const installed = loadRuleCatalog();
  const catalog = worktreeScanCatalog(installed, root, {
    json, rr,
    readJson: path => parseJsonWithoutDuplicateKeys(jsonText.get(path)!, path),
  }, path => loadedRr.load(path, source));
  const pack = catalog.packs.get(WORKTREE_PACK_ID);
  const documents = pack === undefined || !isWorktreePack(pack)
    ? []
    : pack.rules.flatMap(rule => {
      const document = catalog.documents.get(rule.id);
      return document?.kind === "code" ? [document] : [];
    });
  const skipped = catalog.skipped.filter(entry => !installed.skipped.includes(entry)).map((entry): ScanFailure => ({
    code: entry.path.endsWith(".rr") ? "adapter-error" : "rule-left-out",
    message: `${entry.reason}; it does not run`,
    path: `${ref}:${entry.path}`,
    ruleId: null,
  }));
  return { provenance, documents, skipped: [...failures, ...skipped], ...(rr.length === 0 ? {} : { rr: { catalog, source } }) };
}

/**
 * Each `--pack` as the id of a pack installed in the Rerail home, given by its id (`complexity`) or its package name
 * (`@neralabs/rerail-pack-complexity`). One that is not installed is an error: a scan must not pass with its rules
 * silently missing.
 */
function installedPackIds(requested: readonly string[]): string[] {
  if (requested.length === 0) return [];
  const catalog = loadRuleCatalog();
  const installed = [...catalog.packs.values()];
  return requested.map(name => {
    const found = installed.find(pack => pack.id === name || pack.packageName === name || pack.packageName === rulePackPackageOf(name));
    if (found === undefined) {
      const known = installed.map(pack => pack.id).sort().join(", ") || "none";
      throw new Error(`--pack ${name} is not installed in the Rerail home (installed: ${known}); install it with rerail packs add ${name}`);
    }
    return found.id;
  });
}

/**
 * The Rerail home's database, opened read-only and never changed (`openRerailDatabaseAsIs`): a scan from another build
 * must not migrate the installed Rerail's file. Undefined when there is none; a file this build cannot read as it is
 * fails the scan, saying why.
 */
function openHomeDatabase(databasePath: string): RerailDatabase | undefined {
  if (!existsSync(databasePath)) return undefined;
  return openRerailDatabaseAsIs(databasePath);
}

/** Whether the home has Rerail started and set up in `root`: then its configuration is the scan's. */
function homeIsEligible(home: RerailDatabase, root: string): boolean {
  const checkout = lookupCheckout(home, root);
  if (checkout === undefined) return false;
  return stateOf(checkoutStatus(home, checkout)) === "started";
}

/**
 * The tables that say which rules run where, and the approvals: what a scan with the home's configuration copies into
 * its own database. Session history is never read.
 */
const CONFIGURATION_TABLES = ["rules", "rule_catalog_identities", "repositories", "checkouts", "rule_attachments", "rule_settings", "library_rules", "rule_approvals"] as const;

/**
 * Copies `tables` from the home's database into the scan's, column by column (the columns both have), replacing what
 * the scan's database seeded: the scan then runs on the home's configuration without ever writing the home's file.
 */
function copyTables(home: RerailDatabase, scratch: RerailDatabase, tables: readonly string[]): void {
  scratch.$client.pragma("foreign_keys = OFF");
  try {
    scratch.$client.transaction(() => {
      for (const table of tables) {
        const theirs = new Set(tableColumns(home.$client, table));
        const columns = tableColumns(scratch.$client, table).filter(column => theirs.has(column));
        if (columns.length === 0) continue;
        const list = columns.map(column => `"${column}"`).join(", ");
        const insert = scratch.$client.prepare(`INSERT OR REPLACE INTO ${table} (${list}) VALUES (${columns.map(() => "?").join(", ")})`);
        scratch.$client.prepare(`DELETE FROM ${table}`).run();
        for (const row of home.$client.prepare(`SELECT ${list} FROM ${table}`).raw().iterate()) insert.run(...(row as unknown[]));
      }
    })();
  } finally {
    scratch.$client.pragma("foreign_keys = ON");
  }
}

/**
 * Sets up the scan's own database. With the home's configuration (`useHome`), a copy of it. Otherwise `root` recorded,
 * started and set up with `packs` and the ref's rules, and the home's approvals: approval is of content, by its digest,
 * so a rule read from a ref, a worktree's rule or a pack from a folder steers here exactly when a person approved that
 * content in the Rerail home, as it would in a session. Returns the ids of the rules read from the ref.
 */
function prepareScratchDatabase(
  databasePath: string, root: string, home: RerailDatabase | undefined, useHome: boolean,
  packs: readonly string[], refRules: RefRules | undefined, ruleDocuments: Map<string, RuleDocument> | undefined,
): Set<string> {
  const refRuleIds = new Set<string>();
  const database = openRerailDatabase(databasePath);
  try {
    if (useHome && home !== undefined) {
      copyTables(home, database, CONFIGURATION_TABLES);
      return refRuleIds;
    }
    const { repositoryKey } = recordCheckout(database, root);
    const libraryRules: Array<{ sourceKind: "library_rule"; sourceId: string; folder: string }> = [];
    if (refRules !== undefined) {
      const catalog = loadRuleCatalog();
      for (const document of refRules.documents) {
        const stored = upsertLibraryRule(database, catalog, document.rule, "code");
        const ruleId = stored.document.rule.id;
        refRuleIds.add(ruleId);
        ruleDocuments?.set(stored.payloadHash, stored.document);
        libraryRules.push({ sourceKind: "library_rule", sourceId: ruleId, folder: "" });
      }
    }
    setUpRepository(database, repositoryKey, { packs: packs.map(packId => ({ packId, folder: "" })), rules: libraryRules });
    if (refRules !== undefined) {
      // The working tree's own rules are what the change under review may have edited: they do not run.
      setRuleSetting(database, { owner: { kind: "checkout", checkoutPath: root }, folder: "", targetKind: "pack", targetId: WORKTREE_PACK_ID, on: false, mode: null });
    }
    setRepositoryStarted(database, repositoryKey, true);
    if (home !== undefined) copyTables(home, database, ["rule_approvals"]);
  } finally {
    database.$client.close();
  }
  return refRuleIds;
}

function repositoryOf(database: RerailDatabase | undefined, root: string): { key: string; displayName: string } {
  if (database === undefined) return standaloneRepository(root);
  const checkout = lookupCheckout(database, root);
  if (checkout === undefined) return standaloneRepository(root);
  const row = database.select({ displayName: repositories.displayName }).from(repositories)
    .where(eq(repositories.repositoryKey, checkout.repositoryKey)).get();
  return { key: checkout.repositoryKey, displayName: row?.displayName ?? checkout.repositoryKey };
}

/** The matched text is from the evaluated source. A finding without text uses its code. */
function findingFingerprint(ruleId: string, path: string, code: string, matched?: string): string {
  const text = matched?.replace(/\s+/gu, " ").trim() ?? `code:${code}`;
  return createHash("sha256").update(`${ruleId}\0${path}\0${text}`).digest("hex").slice(0, 32);
}

function ruleOf(provenance: AuditRuleProvenance, refRuleIds: ReadonlySet<string>, rulesRef: RefRuleSource | undefined): ScanRule {
  const placement = "placement" in provenance ? provenance.placement : [];
  const mode = (placement.find(entry => entry.folder === "") ?? placement[0])?.mode ?? "log";
  const source = "source" in provenance
    ? { kind: provenance.source.kind as string, id: provenance.source.id, packId: provenance.source.packId }
    : { kind: "unknown", id: provenance.ruleId, packId: null };
  const fromRef = refRuleIds.has(provenance.ruleId) && source.kind === "library_rule" ? rulesRef : undefined;
  return {
    ruleId: provenance.ruleId,
    payloadHash: "payloadHash" in provenance ? provenance.payloadHash : "",
    mode,
    source: fromRef === undefined ? source : { ...fromRef, id: provenance.ruleId, packId: null },
  };
}

/** Converts the engine's position to 1-based. The engine's lines and columns are 0-based (verified by test). */
function position(line: number, column: number): ScanPosition {
  return { line: line + 1, column: column + 1 };
}

/** The checkout `options.repo` is in, once the options that depend on the scope are checked. */
function scanRoot(options: HeadlessScanOptions): string {
  if (options.revision !== undefined && !options.analysisScope?.trim()) {
    throw new Error("--revision requires --analysis-scope <name>");
  }
  if (options.revision === undefined && options.analysisScope !== undefined) {
    throw new Error("--analysis-scope requires --revision");
  }
  const requested = realpathSync(resolve(options.repo));
  const top = gitMaybe(requested, ["rev-parse", "--show-toplevel"]);
  if (options.revision !== undefined && options.scope !== "repository") throw new Error("--revision applies only to --scope repository");
  const bare = options.revision !== undefined && top === null && gitMaybe(requested, ["rev-parse", "--is-bare-repository"]) === "true";
  if (top === null && !bare) throw new Error(`${requested} is not inside a Git checkout`);
  const root = top === null ? requested : realpathSync(top);
  if (options.base !== undefined && options.scope !== "git-diff") throw new Error("--base applies only to --scope git-diff");
  if (options.paths !== undefined && options.scope !== "repository") throw new Error("--path and --paths-from apply only to --scope repository");
  return root;
}

function homeIsScanConfiguration(options: HeadlessScanOptions, home: RerailDatabase | undefined, root: string, packs: readonly string[]): boolean {
  try {
    return home !== undefined && options.rulesRef === undefined && packs.length === 0 && homeIsEligible(home, root);
  } catch (error) {
    home?.$client.close();
    throw error;
  }
}

/** Run with a scratch folder for the audit configuration and remove it afterward. */
async function withScanScratch<T>(home: RerailDatabase | undefined, run: (scratch: string) => Promise<T>): Promise<T> {
  const scratch = mkdtempSync(join(tmpdir(), "rerail-scan-"));
  try {
    return await run(scratch);
  } finally {
    if (home?.$client.open === true) home.$client.close();
    rmSync(scratch, { recursive: true, force: true });
  }
}

/** What the scan covers: the paths asked for, what differs from the base or HEAD, or null for the whole repository. */
function scanTarget(options: HeadlessScanOptions, root: string, source: NativeGitRevisionSource | undefined): Pick<ScanReport, "base" | "paths" | "requestedPaths" | "skippedPaths"> & { failures?: ScanFailure[] } {
  if (options.paths !== undefined) {
    const failures: ScanFailure[] = [];
    const inputs = [...new Set(options.paths.filter(path => path.length > 0).map(path => path.replace(/^(?:\.\/)+/u, "") || "."))];
    const skippedPaths: string[] = [];
    const paths = inputs.filter(path => {
      try {
        if (path === "." || (source === undefined ? existsSync(join(root, path)) : source.containsPath(path))) return true;
        skippedPaths.push(path);
        return false;
      } catch (error) {
        failures.push({
          code: "adapter-error", path, ruleId: null,
          message: error instanceof Error ? error.message : String(error),
        });
        return false;
      }
    }).sort((left, right) => left.localeCompare(right));
    return { base: null, paths: paths.includes(".") ? null : paths, requestedPaths: [...options.paths], skippedPaths, failures };
  }
  if (options.scope !== "git-diff") return { base: null, paths: null };
  if (options.base === undefined) {
    return { base: null, paths: diffPaths(root, gitMaybe(root, ["rev-parse", "--verify", "--quiet", "HEAD"]) === null ? git(root, ["hash-object", "-t", "tree", "/dev/null"]).trim() : "HEAD") };
  }
  const mergeBase = gitMaybe(root, ["merge-base", options.base, "HEAD"]);
  if (mergeBase === null) throw new Error(`--base ${options.base} has no merge base with HEAD in ${root}`);
  return { base: { ref: options.base, mergeBase }, paths: diffPaths(root, mergeBase) };
}

type ScanAudit = AuditInput & { result?: AuditResult };

/** Runs the audits of one session over `paths` (or the whole repository), recording them rather than storing them. */
async function runScanAudits(
  databasePath: string, root: string, sessionId: string, paths: readonly string[] | null,
  revisionSource: NativeGitRevisionSource | undefined, analysisStore: string, analysisRun: NativeAnalysisScopeRun | undefined,
  checkoutRules: (root: string) => RuleCatalog,
): Promise<ScanAudit[]> {
  const audits: ScanAudit[] = [];
  const recorder: AuditRecorder = {
    start(input) { audits.push({ ...input, paths: [...input.paths] }); return String(audits.length - 1); },
    finish(auditId, result) { audits[Number(auditId)]!.result = result; },
    // Progress is not a result: an audit that never finishes stays `interrupted`, and the scan fails.
    progress(auditId, result) { audits[Number(auditId)]!.result = { status: "interrupted", ...result }; },
    interrupt() {},
  };
  const catalogs = new CheckoutCatalogs(databasePath, { checkoutRules });
  const owners = new CheckoutOwners();
  const router = new CheckoutCodeRuleRouter({
    sessionRoot: root,
    startingCheckout: startingCheckout(owners, root).checkoutRoot,
    owners,
    catalogs,
    steerDelivery: { requestSteer: async () => undefined },
    audits: recorder,
    followGitignore: catalogs.followsGitignore(root),
    analysisStore,
    ...(analysisRun === undefined ? {} : { analysisRun }),
    ...(revisionSource === undefined ? {} : { revisionSource }),
  });
  try {
    if (revisionSource !== undefined) {
      const catalog = catalogs.catalog(root);
      if (catalog.status.ruleCount === 0 && catalog.quarantined().length === 0) return audits;
    }
    await router.openWorkspace(sessionId);
    if (paths === null) await router.audit(sessionId, { scope: "repository", mode: "log" });
    else if (paths.length > 0) await router.audit(sessionId, { scope: "touched-files", mode: "log", paths: [...paths] });
  } finally {
    await router.close();
    catalogs.close();
  }
  return audits;
}

function scanRules(audits: readonly ScanAudit[], refRuleIds: ReadonlySet<string>, rulesRef: RefRuleSource | undefined): Map<string, ScanRule> {
  const rules = new Map<string, ScanRule>();
  for (const audit of audits)
    for (const provenance of audit.ruleProvenance ?? []) if (!rules.has(provenance.ruleId)) rules.set(provenance.ruleId, ruleOf(provenance, refRuleIds, rulesRef));
  return rules;
}

function scanViolations(audits: readonly ScanAudit[], rules: ReadonlyMap<string, ScanRule>): ScanViolation[] {
  const violations = audits.flatMap(audit => audit.result?.violations ?? []).map((violation): ScanViolation => ({
    ruleId: violation.ruleId,
    payloadHash: rules.get(violation.ruleId)?.payloadHash ?? null,
    mode: violation.mode,
    code: violation.code,
    clauseId: violation.clauseId,
    path: violation.path ?? null,
    start: violation.span === undefined ? null : position(violation.span.startLine, violation.span.startColumn),
    end: violation.span === undefined ? null : position(violation.span.endLine, violation.span.endColumn),
    message: violation.message,
    fingerprint: violation.fingerprint ?? findingFingerprint(violation.ruleId, violation.path ?? "", violation.code),
  }));
  return distinctFingerprints(violations);
}

function distinctFingerprints(violations: ScanViolation[]): ScanViolation[] {
  // Identical matches of one rule in one file share a text: the second and later take their ordinal too, in the
  // engine's report order, so every finding of a scan has its own fingerprint.
  const seen = new Map<string, number>();
  for (const violation of violations) {
    const count = seen.get(violation.fingerprint) ?? 0;
    seen.set(violation.fingerprint, count + 1);
    if (count > 0) violation.fingerprint = createHash("sha256").update(`${violation.fingerprint}\0${count}`).digest("hex").slice(0, 32);
  }
  return violations;
}

interface RrReport {
  readonly metadata: { policies: readonly { id: string }[] };
  readonly programInputs: readonly { path: string; sha256: string | null }[];
  readonly diagnostics: readonly { policy: string; clause: string; code: string; path: string | null; line: number | null; message: string; span?: { start: ScanPosition; end: ScanPosition; text: string } }[];
}

async function scanRrRules(
  policies: readonly string[], root: string, store: string, output: string, analysisRun: NativeAnalysisScopeRun | undefined,
  paths: string[] | null, source: NativeGitRevisionSource | undefined,
  configured?: { policies: ReadonlyMap<string, RrPolicySource>; mode: ReturnType<typeof resolveRrModes> },
  refPolicy?: RefRules,
  loadedRr?: NativeLoadedRrPolicies,
): Promise<{ rules: ScanRule[]; violations: ScanViolation[] }> {
  const inputs = refPolicy === undefined ? policies.map(path => resolve(path)) : [...policies];
  const reports = JSON.parse(await loadNativeBinding().scanRrPoliciesAsync(inputs, root, store, output, analysisRun, paths ?? undefined, source, refPolicy?.rr?.source, loadedRr)) as RrReport[];
  const rules: ScanRule[] = [];
  const violations: ScanViolation[] = [];
  for (const [index, report] of reports.entries()) {
    const policyPath = inputs[index]!;
    const payloadHash = rrPayloadHash(report.programInputs);
    const provenance = refPolicy === undefined ? { kind: "rr", id: policyPath, packId: null }
      : { ...refPolicy.provenance, id: policyPath, packId: null };
    const modeFor = (id: string, path: string): string | null => {
      if (configured === undefined) return "log";
      const policy = configured.policies.get(id);
      if (policy === undefined || policy.payloadHash !== payloadHash) throw new Error(`${policyPath} changed during the scan`);
      return configured.mode.at(policy, path);
    };
    for (const policy of report.metadata.policies) {
      const mode = modeFor(policy.id, "");
      if (mode !== null) rules.push({ ruleId: policy.id, payloadHash, mode, source: provenance });
    }
    for (const finding of report.diagnostics) {
      const path = finding.path || null;
      const mode = modeFor(finding.policy, path ?? "");
      if (mode === null) continue;
      if (!rules.some(rule => rule.ruleId === finding.policy)) {
        rules.push({ ruleId: finding.policy, payloadHash, mode, source: provenance });
      }
      const line = finding.line !== null && finding.line > 0 ? finding.line : null;
      const start = finding.span?.start ?? (line === null ? null : { line, column: 1 });
      const end = finding.span?.end ?? null;
      const fingerprint = findingFingerprint(finding.policy, path ?? "", finding.code, finding.span?.text);
      violations.push({
        ruleId: finding.policy, payloadHash, mode, code: finding.code, clauseId: finding.clause,
        path, start, end, message: finding.message, fingerprint,
      });
    }
  }
  return { rules, violations: distinctFingerprints(violations) };
}

/** Runs the scan in its scratch folder and reports it: the scan's own database, always, with the home's only read and closed before the scan runs. */
async function scanReport(
  scratch: string, options: HeadlessScanOptions,
  { root, home, useHome, packs, scanId, startedAt, source, analysisStore, analysisRun }: {
    root: string; home: RerailDatabase | undefined; useHome: boolean; packs: readonly string[];
    scanId: string; startedAt: number; source: NativeGitRevisionSource | undefined;
    analysisStore: string;
    analysisRun: NativeAnalysisScopeRun | undefined;
  },
): Promise<ScanReport> {
  const databasePath = join(scratch, "rerail.sqlite");
  const explicitRr = (options.rr?.length ?? 0) > 0;
  const loadedRr = new (loadNativeBinding().LoadedRrPolicies)();
  const refRules = options.rulesRef === undefined ? undefined : rulesAtRef(root, options.rulesRef, loadedRr);
  const installed = loadRuleCatalog();
  const catalog = options.rulesRef !== undefined ? refRules?.rr?.catalog
    : explicitRr ? undefined : worktreeScanCatalog(installed, root, undefined, path => loadedRr.load(path));
  const pinned = new Map<string, RuleCatalog>([[root, refRules === undefined ? catalog ?? installed : installed]]);
  const checkoutRules = (checkout: string): RuleCatalog => {
    const found = pinned.get(checkout);
    if (found !== undefined) return found;
    const loaded = worktreeRuleCatalog(installed, checkout);
    pinned.set(checkout, loaded);
    return loaded;
  };
  const configuredPolicies = catalog?.rrPolicies ?? [];
  const rr = explicitRr ? options.rr! : configuredPolicies.map(policy => policy.path);
  let refRuleIds: Set<string>;
  let repository: ScanReport["repository"];
  try {
    refRuleIds = explicitRr ? new Set() : prepareScratchDatabase(databasePath, root, home, useHome, packs, refRules, options.ruleDocuments);
    const scanDatabase = explicitRr ? undefined : openRerailDatabase(databasePath, { readonly: true });
    try { repository = repositoryOf(scanDatabase ?? home, root); }
    finally { scanDatabase?.$client.close(); }
  } finally {
    home?.$client.close();
  }
  const { failures: pathFailures = [], ...target } = scanTarget(options, root, source);
  const { paths } = target;
  if (source !== undefined && paths !== null) source.restrictPaths(paths);
  let audits = explicitRr ? [] : await runScanAudits(databasePath, root, `scan-${scanId}`, paths, source, analysisStore, analysisRun, checkoutRules);
  const cacheFailure = audits.flatMap(audit => audit.result?.failures ?? []).find(failure => failure.code === "cache-unavailable");
  if (source !== undefined && analysisRun !== undefined && cacheFailure !== undefined) {
    analysisRun.warn(cacheFailure.message);
    analysisStore = ":memory:";
    analysisRun = undefined;
    audits = await runScanAudits(databasePath, root, `scan-${scanId}-retry`, paths, source, analysisStore, analysisRun, checkoutRules);
  }
  const rules = scanRules(audits, refRuleIds, refRules?.provenance);
  const violations = scanViolations(audits, rules);
  const failures = [...pathFailures, ...refRules?.skipped ?? [], ...audits.flatMap(audit => audit.result?.failures ?? []).map((failure): ScanFailure => ({
    code: failure.code,
    message: failure.message,
    path: failure.path ?? null,
    ruleId: failure.ruleId ?? null,
  }))];
  let completed = audits.every(audit => audit.result?.status === "completed")
    && (audits.length > 0 || paths?.length === 0 || source !== undefined)
    && pathFailures.length === 0
    && !refRules?.skipped.some(failure => failure.code === "adapter-error");
  for (const skipped of (refRules === undefined ? catalog?.skipped.filter(policy => policy.path?.endsWith(".rr")) : undefined) ?? []) {
    failures.push({ code: "adapter-error", message: skipped.reason, path: skipped.path, ruleId: null });
    completed = false;
  }
  if (rr.length > 0) {
    try {
      if (explicitRr) for (const path of rr) loadedRr.load(resolve(path));
      let configured: Parameters<typeof scanRrRules>[7];
      if (catalog !== undefined && configuredPolicies.length > 0) {
        const database = openRerailDatabase(databasePath, { readonly: true });
        try {
          const checkout = lookupCheckout(database, root)!;
          configured = {
            policies: new Map(configuredPolicies.map(policy => [policy.id, policy])),
            mode: resolveRrModes(database, catalog, checkout, parentCheckouts(database, checkout).map(parent => parent.checkoutPath), refRules !== undefined),
          };
        } finally {
          database.$client.close();
        }
      }
      const selected = configured === undefined ? rr : configuredPolicies.filter(configured.mode.enabled).map(policy => policy.path);
      const evaluated = await scanRrRules(selected, root, analysisStore, join(scratch, "rr"), analysisRun, paths, source, configured, refRules, loadedRr);
      for (const rule of evaluated.rules) {
        if (rules.has(rule.ruleId)) throw new Error(`duplicate policy ID ${rule.ruleId} across configured rules`);
        rules.set(rule.ruleId, rule);
      }
      violations.push(...evaluated.violations);
      if ((explicitRr || audits.length === 0) && failures.length === 0) completed = true;
    } catch (error) {
      failures.push({ code: "adapter-error", message: error instanceof Error ? error.message : String(error), path: null, ruleId: null });
      completed = false;
    }
  }
  for (const { path, message } of source?.sourceFailures() ?? []) {
    failures.push({ code: "adapter-error", message, path, ruleId: null });
    completed = false;
  }
  return {
    schema: SCAN_SCHEMA,
    scanId,
    tool: { name: "rerail", version: options.version },
    startedAt,
    finishedAt: Date.now(),
    repository,
    checkout: {
      path: root,
      head: gitMaybe(root, ["rev-parse", "--verify", "--quiet", "HEAD"]),
      ref: gitMaybe(root, ["symbolic-ref", "--quiet", "HEAD"]),
    },
    ...(source === undefined || options.revision === undefined ? {} : { source: { revision: options.revision, tree: source.tree, commit: source.commit ?? null } }),
    scope: options.scope,
    ...target,
    rulesSource: explicitRr ? { kind: "rr" } : refRules?.provenance ?? { kind: useHome ? "home" : "worktree" },
    status: completed ? "completed" : "failed",
    rules: [...rules.values()].sort((left, right) => left.ruleId.localeCompare(right.ruleId)),
    violations,
    failures,
    summary: { violations: violations.length, failures: failures.length },
  };
}

/** Runs one headless scan. Throws on bad input (not a checkout, unknown ref); an audit that fails is `status: "failed"`. */
export async function runHeadlessScan(options: HeadlessScanOptions): Promise<ScanReport> {
  if (options.rr?.length && (options.packs.length > 0 || options.rulesRef !== undefined)) {
    throw new Error("--rr selects explicit policy files. Use it without --pack or --rules-ref.");
  }
  const startedAt = Date.now();
  const binding = loadNativeBinding();
  const homeDatabase = options.databasePath ?? defaultRerailDatabasePath();
  const directory = options.revision === undefined;
  const scanId = randomUUID();
  const root = scanRoot(options);
  const durableStore = scanContentStore(homeDatabase);
  assertContentStoreOwner(durableStore);
  const packs = installedPackIds(options.packs);
  let analysisRun: NativeAnalysisScopeRun | undefined;
  let cacheWarning: string | undefined;
  try {
    mkdirSync(dirname(homeDatabase), { recursive: true, mode: 0o700 });
    const generation = directory ? await acquireDirectoryAnalysisGeneration(homeDatabase, resolve(options.repo), durableStore) : 0;
    analysisRun = new binding.AnalysisScopeRun(durableStore, options.analysisScope ?? resolve(options.repo), directory, scanId, generation, directory ? analysisScopeHome(homeDatabase) : undefined);
  } catch (error) {
    cacheWarning = error instanceof Error ? error.message : String(error);
  }
  const analysisStore = analysisRun === undefined ? ":memory:" : durableStore;
  try {
    const source = options.revision === undefined ? undefined : new binding.GitRevisionSource(root, options.revision, options.analysisScope!);
    if (source !== undefined)
      source.setSourceLanguages(JSON.stringify({ suffixes: Object.fromEntries(SOURCE_LANGUAGES), names: Object.fromEntries(SOURCE_NAMES) }));
    const home = openHomeDatabase(homeDatabase);
    const useHome = homeIsScanConfiguration(options, home, root, packs);
    const report = await withScanScratch(home, scratch => scanReport(scratch, options, { root, home, useHome, packs, scanId, startedAt, source, analysisStore, analysisRun }));
    if (cacheWarning !== undefined) report.cacheWarnings = [cacheWarning];
    try {
      const warning = analysisRun?.finish(report.status === "completed");
      if (warning !== null && warning !== undefined) report.cacheWarnings = [warning];
    } catch (error) {
      (report.cacheWarnings ??= []).push(error instanceof Error ? error.message : String(error));
      try { analysisRun?.finish(false); } catch { /* Collection can release any abandoned ownership. */ }
    }
    return report;
  } catch (error) {
    try {
      analysisRun?.finish(false);
    } catch (cacheError) {
      process.stderr.write(`rerail: analysis cache cleanup failed: ${String(cacheError)}\n`);
    }
    throw error;
  } finally {
    if (analysisRun !== undefined) await binding.collectAnalysisCacheAsync(durableStore).catch(error => {
      process.stderr.write(`rerail: analysis cache collection failed: ${String(error)}\n`);
    });
  }
}

/** One line per violation, then a summary: the text `rerail scan` prints without `--json`. */
export function describeScan(report: ScanReport): string {
  const lines = report.violations.map(violation => {
    const at = violation.path === null ? "" : `${violation.path}${violation.start === null ? "" : `:${violation.start.line}:${violation.start.column}`} `;
    return `${at}[${violation.ruleId}] ${violation.message}`;
  });
  for (const failure of report.failures) lines.push(`failure ${failure.code}${failure.path === null ? "" : ` ${failure.path}`}: ${failure.message}`);
  for (const warning of report.cacheWarnings ?? []) lines.push(`Cache was not retained. ${warning}`);
  const scope = report.scope === "git-diff" ? `git-diff${report.base === null ? "" : ` against ${report.base.ref}`}` : "repository";
  if (report.source !== undefined) {
    lines.push(`Source ${report.source.commit === null ? "tree " + report.source.tree : "commit " + report.source.commit}`);
  }
  lines.push(`Scan ${report.status}: ${report.repository.displayName} (${scope}), ${report.rules.length} rules, ${report.summary.violations} violations, ${report.summary.failures} failures`);
  return lines.join("\n");
}
