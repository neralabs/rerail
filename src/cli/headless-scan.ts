/**
 * Internal testing: the headless `rerail scan` (no `--session`). It audits one checkout in this process, with no daemon
 * and no session, and reports a stable JSON document (`rerail.scan/1`, docs/headless-scan.md) a CI job or another tool
 * can read. Nothing is added to the Rerail database's session history: the audit is recorded in memory. (With the
 * home's configuration it opens that database as the daemon would, which applies this version's schema to it.)
 *
 * Which rules run:
 * - When the scan's tree declares its packs (`.rerail/packs.json`, in the working tree or at `--rules-ref`) and no
 *   `--pack` is given, exactly those packs, fetched or read for this scan (`scan-packs.ts`), with the option values
 *   the file sets for their rules, and the tree's own rules, on a throwaway database: the Rerail home's attachments,
 *   settings and installed packs are not used, so every machine runs the same rules for the same commit.
 * - Otherwise the Rerail home's configuration for the checkout, when Rerail is started and set up in it there (packs
 *   attached, approvals, settings), the tree declares no packs, and neither `--pack` nor `--rules-ref` is given.
 * - Otherwise a throwaway database: the checkout's own `.rerail/rules/` (unapproved, so they run in Log), plus each
 *   `--pack` installed in the Rerail home. `--pack` replaces the packs `.rerail/packs.json` declares, which is then not
 *   read: a scan of experimental packs. With `--rules-ref <ref>` the rules come from `.rerail/rules/` at that Git ref
 *   instead of the working tree, which the change under review cannot edit. Either way the approval policy is a
 *   session's: content no one approved in the Rerail home (`rerail rules approve`) runs in Log.
 */
import { createHash, randomUUID } from "node:crypto";
import { type Dirent, existsSync, lstatSync, mkdirSync, mkdtempSync, readdirSync, realpathSync, rmSync, statSync } from "node:fs";
import { tmpdir } from "node:os";
import { basename, dirname, join, parse, posix, relative, resolve, sep } from "node:path";
import { eq } from "drizzle-orm";
import type { AuditInput, AuditRecorder, AuditResult, AuditRuleProvenance } from "../engine/audit/audit.js";
import { CheckoutOwners } from "../engine/catalog/checkout-owner.js";
import { textRealPath } from "../engine/catalog/checkout-control.js";
import { checkoutStatus, lookupCheckout, parentCheckouts, recordCheckout, standaloneRepository } from "../engine/catalog/repository-registry.js";
import type { RrPolicySource, RuleCatalog, RuleSetting } from "../engine/catalog/repository-model.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { rulePackPackageOf } from "../engine/catalog/rule-packs.js";
import { resolveRrModes, setRuleSetting, setUpRepository } from "../engine/catalog/rule-configuration.js";
import { upsertLibraryRule } from "../engine/catalog/rule-library.js";
import type { DeclaredRuleConfig } from "../engine/catalog/packs-file/pack-config.js";
import type { RuleDocument } from "../engine/catalog/rule-store.js";
import { isWorktreePack, MAX_RULE_DOCUMENT_BYTES, rrPayloadHash, WORKTREE_PACK_ID, WORKTREE_RULES_FOLDER, worktreeRuleCatalog, worktreeScanCatalog } from "../engine/catalog/worktree-pack.js";
import { CheckoutCodeRuleRouter } from "../engine/code-rule/checkout-code-rule-router.js";
import { CheckoutCatalogs, type CheckoutEligibility, startingCheckout } from "../engine/code-rule/code-rule-dsl-catalog.js";
import { loadNativeBinding, type GitPathIdentity, type GitPathSkipReason, type NativeAnalysisScopeRun, type NativeGitRevisionSource, type NativeLoadedRrPolicies } from "../engine/native/native.js";
import { acquireDirectoryAnalysisRegistration, acquireNamedAnalysisRun, analysisScopeHome, finishNamedAnalysisRegistration, scanContentStore } from "../engine/native/analysis-scope.js";
import { assertContentStoreOwner } from "../engine/native/main-instance.js";
import { SOURCE_LANGUAGES, SOURCE_NAMES } from "../engine/code-rule/code-rule-languages.js";
import { runGit, runGitBytes } from "../engine/git.js";
import { isUtf8 } from "node:buffer";
import { parseJsonWithoutDuplicateKeys } from "../engine/json-duplicate-keys.js";
import { compareDeterministicStrings } from "../engine/stable-json.js";
import { defaultRerailDatabasePath, openRerailDatabase, openRerailDatabaseAsIs, type RerailDatabase } from "../engine/storage/database.js";
import { repositories } from "../engine/storage/schema.js";
import { tableColumns } from "../engine/storage/table-probe.js";
import { setRepositoryStarted, stateOf } from "../runtime/repository-start.js";
import type { PackStore } from "../setup/rule-pack-store.js";
import { declaredPacksOf, declaredScanPacks, type DeclaredPacks, type DeclaredScanPacks, type ScanPack, type ScanPackAttachment } from "./scan-packs.js";

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

export interface GitCommitLocation {
  kind: "git-commit";
  repository: string;
  commit: string;
  field: string | null;
}

export interface ScanViolation {
  ruleId: string;
  payloadHash: string | null;
  mode: string;
  code: string;
  clauseId: string;
  path: string | null;
  location?: GitCommitLocation;
  pathIdentity?: GitPathIdentity;
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
  pathIdentity?: GitPathIdentity;
  ruleId: string | null;
}

const textPath = (value: string): GitPathIdentity => ({ encoding: "utf8", value });

/** The headless scan's report: contract `rerail.scan/1`. */
export interface ScanReport {
  schema: typeof SCAN_SCHEMA;
  scanId: string;
  tool: { name: "rerail"; version: string };
  startedAt: number;
  finishedAt: number;
  repository: { key: string; displayName: string };
  checkout: { path: string; head: string | null; ref: string | null; refHex?: string };
  source?: { revision: string; tree: string; commit: string | null };
  scope: "repository" | "git-diff";
  analysisScope: { kind: "named"; name: string } | { kind: "directory"; path: string };
  cache: { publication: NonNullable<NativeAnalysisScopeRun["publication"]>; collection: "completed" | "failed" | "skipped" };
  base: { ref: string; mergeBase: string } | null;
  paths: string[] | null;
  requestedPaths?: string[];
  skippedPaths?: string[];
  skippedPathDetails?: { path: string; reason: GitPathSkipReason; mode?: number }[];
  /** Null when the scan had something to scan and ran no audit, so no configuration's rules ran: `failures` says why. */
  rulesSource: { kind: "home" } | { kind: "worktree" } | RefRuleSource | { kind: "rr" } | null;
  /** The rule packs the scan ran, besides the tree's own rules: what `.rerail/packs.json` lists, else the home's (its configuration or `--pack`). */
  packs: ScanPack[];
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
  /** Installs the published packs `.rerail/packs.json` lists into a scan's own home; npm (`npmPackStore`) by default. */
  packStore?: (home: string) => PackStore;
}

function gitBytes(cwd: string, args: readonly string[]): Buffer {
  const result = runGitBytes(cwd, args, { maxBuffer: 256 * 1024 * 1024 });
  if (result.error !== undefined) throw result.error;
  if (result.status !== 0) throw new Error(`git ${args.join(" ")} failed: ${result.stderr.trim()}`);
  return result.stdout;
}

function gitMaybe(cwd: string, args: readonly string[]): string | null {
  const result = runGit(cwd, args);
  return result.status === 0 ? result.stdout.replace(/\n$/u, "") : null;
}

function checkoutRef(root: string): Pick<ScanReport["checkout"], "ref" | "refHex"> {
  const result = runGitBytes(root, ["symbolic-ref", "--quiet", "HEAD"]);
  if (result.status !== 0) return { ref: null };
  const bytes = result.stdout.at(-1) === 10 ? result.stdout.subarray(0, -1) : result.stdout;
  return isUtf8(bytes) ? { ref: bytes.toString("utf8") } : { ref: null, refHex: bytes.toString("hex") };
}

/**
 * Changed files against `against` (the working tree, staged changes included) plus untracked files, sorted. With
 * `--no-renames` a rename is its new path, and Git needs no blob it would fetch to detect one: lazy fetch is off.
 */
function diffPaths(root: string, against: string): { paths: string[]; failures: ScanFailure[] } {
  const paths = new Set<string>();
  const invalid = new Set<string>();
  for (const args of [
    ["diff", "--name-only", "--no-renames", "--relative", "--diff-filter=ACMRTUXB", "-z", against, "--"],
    ["ls-files", "--others", "--exclude-standard", "-z", "--"],
  ]) {
    const bytes = gitBytes(root, args);
    let start = 0;
    for (let end = bytes.indexOf(0); end >= 0; end = bytes.indexOf(0, start)) {
      const name = bytes.subarray(start, end);
      if (isUtf8(name)) paths.add(name.toString("utf8"));
      else invalid.add(name.toString("hex"));
      start = end + 1;
    }
  }
  return {
    paths: [...paths].sort((left, right) => left.localeCompare(right)),
    failures: [...invalid].sort().map(hex => ({
      code: "source-incomplete", path: `[hex:${hex}]`, pathIdentity: { encoding: "hex", value: hex }, message: "Git path has invalid UTF-8", ruleId: null,
    })),
  };
}

/** A rule read from `--rules-ref`, and what loading the ref's rules left out (each with why), as a session would. */
interface RefRules {
  readonly provenance: RefRuleSource;
  readonly documents: RuleDocument[];
  readonly skipped: ScanFailure[];
  readonly rr?: { catalog: RuleCatalog; source: NativeGitRevisionSource };
}

/** Load the ref's policies through the same catalog rules used for a checkout. */
function rulesAtRef(root: string, ref: string, loadedRr: NativeLoadedRrPolicies, invocation: string, installed: RuleCatalog): RefRules {
  const binding = loadNativeBinding();
  let source: NativeGitRevisionSource;
  try {
    source = new binding.GitRevisionSource(root, ref, "rules-ref", invocation);
  } catch (error) {
    const detail = error instanceof Error ? error.message : String(error);
    throw new Error(`Cannot resolve --rules-ref ${ref} in ${root}. ${detail}`, { cause: error });
  }
  const commit = source.commit;
  if (!commit) throw new Error(`--rules-ref ${ref} resolved to tree ${source.tree}. This option requires a commit.`);
  const provenance: RefRuleSource = { kind: "ref", ref, commit };
  const failures: ScanFailure[] = [];
  const unreadable = (path: string, error: unknown, pathIdentity = textPath(path)) => {
    failures.push({
      code: "adapter-error", path: `${ref}:${path}`, pathIdentity, ruleId: null,
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
      if (nameBytes !== undefined && (bytes?.endsWith(".json") || bytes?.endsWith(".rerail"))) {
        const parent = WORKTREE_RULES_FOLDER + "/";
        unreadable(`${parent}[hex:${nameBytes.toString("hex")}]`, new Error("Git policy name has invalid UTF-8"), {
          encoding: "hex", value: Buffer.concat([Buffer.from(parent), nameBytes]).toString("hex"),
        });
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
      const contents = source.readSourceChecked(file.path, MAX_RULE_DOCUMENT_BYTES);
      if (contents.status === "absent") throw new Error("Git tree no longer contains the listed policy");
      if (contents.status === "too-large") file.size = MAX_RULE_DOCUMENT_BYTES + 1;
      else {
        try {
          jsonText.set(file.path, utf8.decode(contents.bytes));
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
  const rr = files.filter(file => file.name.endsWith(".rerail"));
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
    code: entry.path.endsWith(".rerail") ? "adapter-error" : "rule-left-out",
    message: `${entry.reason}; it does not run`,
    path: `${ref}:${entry.path}`,
    pathIdentity: textPath(entry.path),
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
 * Sets up the scan's own database. With the home's configuration (`useHome`), a copy of it with `root` recorded, as a
 * session started there records its folder. Otherwise a throwaway configuration (`setUpThrowawayConfiguration`).
 * Returns the ids of the rules read from the ref.
 */
function prepareScanConfiguration(
  database: RerailDatabase, root: string, home: RerailDatabase | undefined, useHome: boolean,
  packs: readonly ScanPackAttachment[], ruleConfigs: readonly DeclaredRuleConfig[], refRules: RefRules | undefined,
  ruleDocuments: Map<string, RuleDocument> | undefined, catalog: RuleCatalog,
): Set<string> {
  if (!useHome || home === undefined) return setUpThrowawayConfiguration(database, root, home, packs, ruleConfigs, refRules, ruleDocuments, catalog);
  copyTables(home, database, CONFIGURATION_TABLES);
  // A copy of a started checkout made with its `.git` has no row: the home finds it by its clone id, but no session
  // recorded it. Recorded here, it runs the home's rules as a session started in it would.
  recordCheckout(database, root);
  return new Set();
}

/**
 * `root` recorded, started and set up with `packs`, each rule's `ruleConfigs` as its whole-repository `config`
 * setting, the ref's rules, and the home's approvals: approval is of content, by its digest, so a rule read from a
 * ref, a worktree's rule or a pack from a folder steers here exactly when a person approved that content in the
 * Rerail home, as it would in a session. The ref's rules are stored against `catalog`, the packs the scan runs.
 * Returns the ids of the rules read from the ref.
 */
function setUpThrowawayConfiguration(
  database: RerailDatabase, root: string, home: RerailDatabase | undefined,
  packs: readonly ScanPackAttachment[], ruleConfigs: readonly DeclaredRuleConfig[], refRules: RefRules | undefined,
  ruleDocuments: Map<string, RuleDocument> | undefined, catalog: RuleCatalog,
): Set<string> {
  const refRuleIds = new Set<string>();
  const { repositoryKey } = recordCheckout(database, root);
  const libraryRules: Array<{ sourceKind: "library_rule"; sourceId: string; folder: string }> = [];
  if (refRules !== undefined) {
    for (const document of refRules.documents) {
      const stored = upsertLibraryRule(database, catalog, document.rule, "code");
      const ruleId = stored.document.rule.id;
      refRuleIds.add(ruleId);
      ruleDocuments?.set(stored.payloadHash, stored.document);
      libraryRules.push({ sourceKind: "library_rule", sourceId: ruleId, folder: "" });
    }
  }
  const owner = { kind: "repository" as const, repositoryKey };
  const settings = ruleConfigs.map(({ ruleId, config }): RuleSetting => ({ owner, folder: "", targetKind: "rule", targetId: ruleId, on: null, mode: null, config }));
  setUpRepository(database, repositoryKey, { packs: [...packs], rules: libraryRules, settings });
  if (refRules !== undefined) {
    // The working tree's own rules are what the change under review may have edited: they do not run.
    setRuleSetting(database, { owner: { kind: "checkout", checkoutPath: root }, folder: "", targetKind: "pack", targetId: WORKTREE_PACK_ID, on: false, mode: null });
  }
  setRepositoryStarted(database, repositoryKey, true);
  if (home !== undefined) copyTables(home, database, ["rule_approvals"]);
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
function scanRoot(options: HeadlessScanOptions): { root: string; requested: string } {
  if (options.revision !== undefined && !options.analysisScope?.trim()) {
    throw new Error("--revision requires --analysis-scope <name>");
  }
  if (options.revision === undefined && options.analysisScope !== undefined) {
    throw new Error("--analysis-scope requires --revision");
  }
  const requested = textRealPath(resolve(options.repo));
  const top = gitMaybe(requested, ["rev-parse", "--show-toplevel"]);
  if (options.revision !== undefined && options.scope !== "repository") throw new Error("--revision applies only to --scope repository");
  const bare = options.revision !== undefined && top === null && gitMaybe(requested, ["rev-parse", "--is-bare-repository"]) === "true";
  if (top === null && !bare) throw new Error(`${requested} is not inside a Git checkout`);
  const root = top === null ? requested : textRealPath(top);
  if (options.base !== undefined && options.scope !== "git-diff") throw new Error("--base applies only to --scope git-diff");
  if (options.paths !== undefined && options.scope !== "repository") throw new Error("--path and --paths-from apply only to --scope repository");
  return { root, requested };
}

/** Get the actual name of a link itself. A realpath of the link would name its target. */
function worktreeLinkName(parent: string, part: string, ino: bigint, dev: bigint): string {
  let entries: Dirent<Buffer>[];
  try { entries = readdirSync(parent, { encoding: "buffer", withFileTypes: true }); }
  catch (error) {
    // A searchable directory can allow a named lookup without allowing a listing.
    if (["EACCES", "EPERM"].includes((error as NodeJS.ErrnoException).code ?? "")) return part;
    throw error;
  }
  const requested = Buffer.from(part), folded = part.normalize().toLowerCase();
  if (entries.some(entry => entry.name.equals(requested))) return part;
  const aliases = entries.filter(entry => entry.isSymbolicLink() && isUtf8(entry.name))
    .map(entry => entry.name.toString()).filter(name => {
      const entry = lstatSync(join(parent, name), { bigint: true, throwIfNoEntry: false });
      return entry?.ino === ino && entry.dev === dev;
    });
  return aliases.length === 1 ? aliases[0]! : aliases.find(name => name.normalize().toLowerCase() === folded) ?? part;
}

/** Match filesystem spelling while keeping a selected link distinct from a selection of its target. */
function worktreeSelection(root: string, path: string, directory: boolean): string | undefined {
  const requested = join(root, path);
  if (!existsSync(requested) || (directory && !statSync(requested).isDirectory())) return undefined;
  if (realpathSync.native(requested, { encoding: "buffer" }).equals(Buffer.from(requested))) return path;
  let candidate = root;
  return path.split("/").map(part => {
    const parent = candidate;
    candidate = join(parent, part);
    const entry = lstatSync(candidate, { bigint: true });
    return entry.isSymbolicLink() ? worktreeLinkName(parent, part, entry.ino, entry.dev) : basename(realpathSync.native(candidate));
  }).join("/");
}

/** What the scan covers: the paths asked for, what differs from the base or HEAD, or null for the whole repository. */
function scanTarget(options: HeadlessScanOptions, root: string, source: NativeGitRevisionSource | undefined): Pick<ScanReport, "base" | "paths" | "requestedPaths" | "skippedPaths" | "skippedPathDetails"> & { failures?: ScanFailure[] } {
  if (options.paths !== undefined) {
    const failures: ScanFailure[] = [];
    const skippedPaths: string[] = [];
    const skippedPathDetails: NonNullable<ScanReport["skippedPathDetails"]> = [];
    const selected = new Set<string>();
    for (const requested of new Set(options.paths)) {
      if (requested.length === 0) continue;
      const spelling = sep === "\\" ? requested.replaceAll("\\", "/") : requested;
      const path = posix.normalize(spelling).replace(/\/$/u, "") || ".";
      if (parse(requested).root || path === ".." || path.startsWith("../")) {
        throw new Error(`Expected a repository-relative scan path within the repository: ${requested}`);
      }
      if (path === ".") {
        selected.add(path);
        continue;
      }
      const directory = spelling.endsWith("/") || spelling.endsWith("/.") || spelling.endsWith("/..");
      const lookup = directory ? path + "/" : path;
      try {
        if (source === undefined) {
          const name = worktreeSelection(root, path, directory);
          if (name === undefined) skippedPaths.push(requested);
          else selected.add(name);
        } else {
          const { status, mode } = source.pathStatus(lookup);
          if (status === "selected") selected.add(path);
          else {
            skippedPaths.push(requested);
            skippedPathDetails.push({ path: requested, reason: status, ...(mode === undefined ? {} : { mode }) });
          }
        }
      } catch (error) {
        failures.push({
          code: "adapter-error", path: requested, ruleId: null,
          message: error instanceof Error ? error.message : String(error),
        });
      }
    }
    const paths = selected.has(".") ? null : [...selected].sort((left, right) => left.localeCompare(right));
    return { base: null, paths, requestedPaths: [...options.paths], skippedPaths, ...(source && { skippedPathDetails }), failures };
  }
  if (options.scope !== "git-diff") return { base: null, paths: null };
  if (options.base === undefined) {
    const against = gitMaybe(root, ["rev-parse", "--verify", "--quiet", "HEAD"]) ?? gitBytes(root, ["hash-object", "-t", "tree", "--stdin"]).toString("ascii").trim();
    return { base: null, ...diffPaths(root, against) };
  }
  const mergeBase = gitMaybe(root, ["merge-base", options.base, "HEAD"]);
  if (mergeBase === null) throw new Error(`--base ${options.base} has no merge base with HEAD in ${root}`);
  return { base: { ref: options.base, mergeBase }, ...diffPaths(root, mergeBase) };
}

type ScanAudit = AuditInput & { result?: AuditResult };

/**
 * Runs the audits of one session over `paths` (or the whole repository), recording them rather than storing them, on
 * the catalog `checkoutRules` gives each checkout. When there was something to scan and no audit ran, `unaudited`
 * says why for each checkout the scan meant to audit.
 */
async function runScanAudits(
  database: RerailDatabase, databasePath: string, root: string, sessionId: string, paths: readonly string[] | null,
  revisionSource: NativeGitRevisionSource | undefined, analysisStore: string, analysisRun: NativeAnalysisScopeRun | undefined,
  checkoutRules: (root: string) => RuleCatalog,
): Promise<{ audits: ScanAudit[]; unaudited: ScanFailure[] }> {
  const audits: ScanAudit[] = [];
  const recorder: AuditRecorder = {
    start(input) { audits.push({ ...input, paths: [...input.paths] }); return String(audits.length - 1); },
    finish(auditId, result) { audits[Number(auditId)]!.result = result; },
    // Progress is not a result: an audit that never finishes stays `interrupted`, and the scan fails.
    progress(auditId, result) { audits[Number(auditId)]!.result = { status: "interrupted", ...result }; },
    interrupt() {},
  };
  const catalogs = new CheckoutCatalogs(databasePath, { checkoutRules, database });
  const owners = new CheckoutOwners(catalogs.database);
  const start = startingCheckout(owners, root).checkoutRoot;
  const router = new CheckoutCodeRuleRouter({
    sessionRoot: root,
    startingCheckout: start,
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
      if (catalog.status.ruleCount === 0 && catalog.quarantined().length === 0) return { audits, unaudited: [] };
    }
    await router.openWorkspace(sessionId);
    if (paths === null) await router.audit(sessionId, { scope: "repository", mode: "log" });
    else if (paths.length > 0) await router.audit(sessionId, { scope: "touched-files", mode: "log", paths: [...paths] });
    const nothingRan = audits.length === 0 && paths?.length !== 0;
    return { audits, unaudited: nothingRan ? unauditedCheckouts(router, start, paths) : [] };
  } finally {
    await router.close();
    catalogs.close();
  }
}

/**
 * A `checkout-not-audited` failure for each checkout a scan that ran no audit meant to audit: the one it starts in, or
 * those that own `paths` (a submodule's own, say), each with why the router runs no rules there.
 */
function unauditedCheckouts(router: CheckoutCodeRuleRouter, start: string, paths: readonly string[] | null): ScanFailure[] {
  const owners = (paths ?? []).map(path => router.sourceRoot(path)).filter((owner): owner is string => owner !== undefined);
  const checkouts = owners.length === 0 ? [start] : [...new Set(owners)].sort((left, right) => left.localeCompare(right));
  return checkouts.map(checkout => notAudited(checkout, router.whyNoRules(checkout)));
}

/** Why the router runs no rules in a checkout, as that checkout's eligibility says. */
const NO_RULES_BECAUSE: Record<Exclude<CheckoutEligibility, "ok">, (checkout: string) => string> = {
  unrecorded: checkout => `Rerail has no record of ${checkout}`,
  stopped: checkout => `Rerail is stopped in ${checkout}`,
  "not-set-up": checkout => `Rerail is not set up in ${checkout}'s repository`,
};

/** The failure that stands for a checkout a scan meant to audit and did not, so no rules ran there. */
function notAudited(checkout: string, why: ReturnType<CheckoutCodeRuleRouter["whyNoRules"]>): ScanFailure {
  const reason = why === undefined ? "Rerail runs in it, but no audit started" : NO_RULES_BECAUSE[why.eligibility](why.checkoutRoot);
  return { code: "checkout-not-audited", message: `${checkout} was not audited, so no rules ran: ${reason}`, path: null, ruleId: null };
}

/** The home's packs that `rules` came from, for a scan that ran the home's configuration or `--pack`. */
function homePacks(rules: ReadonlyMap<string, ScanRule>): ScanPack[] {
  const ids = new Set([...rules.values()].flatMap(rule => rule.source.packId === null || rule.source.packId === WORKTREE_PACK_ID ? [] : [rule.source.packId]));
  const installed = loadRuleCatalog();
  return [...ids].sort().flatMap(id => {
    const pack = installed.packs.get(id);
    return pack === undefined ? [] : [{ id, packageName: pack.packageName, version: pack.version, from: "home" as const }];
  });
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
  readonly diagnostics: readonly { policy: string; clause: string; code: string; path: string | null; line: number | null; message: string; location?: GitCommitLocation; span?: { start: ScanPosition; end: ScanPosition; text: string } }[];
}

async function scanRrRules(
  policies: readonly string[], root: string, store: string, output: string, analysisRun: NativeAnalysisScopeRun | undefined,
  paths: string[] | null, source: NativeGitRevisionSource | undefined,
  configured?: { policies: ReadonlyMap<string, RrPolicySource>; mode: ReturnType<typeof resolveRrModes> },
  refPolicy?: RefRules,
  loadedRr?: NativeLoadedRrPolicies,
): Promise<{ rules: ScanRule[]; violations: ScanViolation[]; failures: ScanFailure[] }> {
  const inputs = refPolicy === undefined ? policies.map(path => resolve(path)) : [...policies];
  const reports = JSON.parse(await loadNativeBinding().scanRrPoliciesAsync(inputs, root, store, output, analysisRun, paths ?? undefined, source, refPolicy?.rr?.source, loadedRr)) as (RrReport | { error: string; ruleId: string | null })[];
  const rules: ScanRule[] = [];
  const violations: ScanViolation[] = [];
  const failures: ScanFailure[] = [];
  for (const [index, report] of reports.entries()) {
    const policyPath = inputs[index]!;
    if ("error" in report) {
      failures.push({ code: "adapter-error", path: policyPath, ruleId: report.ruleId, message: report.error });
      continue;
    }
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
      const location = finding.location;
      const fingerprint = location === undefined ? findingFingerprint(finding.policy, path ?? "", finding.code, finding.span?.text)
        : createHash("sha256").update(JSON.stringify([location.kind, finding.policy, finding.clause, finding.code,
          location.repository, location.commit, location.field])).digest("hex").slice(0, 32);
      violations.push({
        ruleId: finding.policy, payloadHash, mode, code: finding.code, clauseId: finding.clause,
        path, start, end, message: finding.message, fingerprint, ...(location && { location }),
      });
    }
  }
  return { rules, violations: distinctFingerprints(violations), failures };
}

/** `run` with the packs `declared` lists installed in a scratch folder of the scan's own, removed after. */
async function withDeclaredPacks<T>(
  root: string, declared: DeclaredPacks | undefined, packStore: HeadlessScanOptions["packStore"],
  run: (packs: DeclaredScanPacks | undefined) => Promise<T>,
): Promise<T> {
  if (declared === undefined) return run(undefined);
  const scratch = mkdtempSync(join(tmpdir(), "rerail-scan-"));
  try {
    return await run(await declaredScanPacks(root, declared, scratch, packStore));
  } finally {
    rmSync(scratch, { recursive: true, force: true });
  }
}

/**
 * Run with a private in-memory copy of the configuration and the selected content store, on the packs the tree
 * declares (`declaredPacks`) when it lists them, else the installed packs.
 */
async function scanReport(
  database: RerailDatabase, options: HeadlessScanOptions,
  { root, home, packs, declaredPacks, scanId, startedAt, source, analysisStore, analysisRun }: {
    root: string; home: RerailDatabase | undefined; packs: readonly string[]; declaredPacks: DeclaredScanPacks | undefined;
    scanId: string; startedAt: number; source: NativeGitRevisionSource | undefined;
    analysisStore: string;
    analysisRun: NativeAnalysisScopeRun | undefined;
  },
): Promise<Omit<ScanReport, "analysisScope" | "cache">> {
  const databasePath = options.databasePath ?? defaultRerailDatabasePath();
  const explicitRr = (options.rr?.length ?? 0) > 0;
  const loadedRr = new (loadNativeBinding().LoadedRrPolicies)(analysisStore);
  const installed = declaredPacks?.catalog ?? loadRuleCatalog();
  const refRules = options.rulesRef === undefined ? undefined : rulesAtRef(root, options.rulesRef, loadedRr, resolve(options.repo), installed);
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
  const readConfiguration = () => {
    const useHome = declaredPacks === undefined && home !== undefined && options.rulesRef === undefined && packs.length === 0 && homeIsEligible(home, root);
    const attached = declaredPacks?.attachments ?? packs.map(packId => ({ packId, folder: "" }));
    const refRuleIds = explicitRr ? new Set<string>() : prepareScanConfiguration(database, root, home, useHome, attached, declaredPacks?.ruleConfigs ?? [], refRules, options.ruleDocuments, installed);
    return { useHome, refRuleIds, repository: repositoryOf(explicitRr ? home : database, root) };
  };
  const { useHome, refRuleIds, repository } = home === undefined ? readConfiguration() : home.$client.transaction(readConfiguration)();
  home?.$client.close();
  const { failures: pathFailures = [], ...target } = scanTarget(options, root, source);
  const { paths } = target;
  if (source !== undefined && paths !== null) source.restrictPaths(paths);
  let { audits, unaudited } = explicitRr ? { audits: [], unaudited: [] } : await runScanAudits(database, databasePath, root, `scan-${scanId}`, paths, source, analysisStore, analysisRun, checkoutRules);
  const cacheFailure = audits.flatMap(audit => audit.result?.failures ?? []).find(failure => failure.code === "cache-unavailable");
  if (source !== undefined && analysisRun !== undefined && cacheFailure !== undefined) {
    analysisRun.warn(cacheFailure.message);
    analysisStore = ":memory:";
    analysisRun = undefined;
    ({ audits, unaudited } = await runScanAudits(database, databasePath, root, `scan-${scanId}-retry`, paths, source, analysisStore, analysisRun, checkoutRules));
  }
  // `.rerail` policies are the checkout's rules too: a scan that runs them audited it.
  if (rr.length > 0) unaudited = [];
  const audited = unaudited.length === 0;
  const rules = scanRules(audits, refRuleIds, refRules?.provenance);
  const violations = scanViolations(audits, rules);
  const failures = [...pathFailures, ...refRules?.skipped ?? [], ...audits.flatMap(audit => audit.result?.failures ?? []).map((failure): ScanFailure => ({
    code: failure.code,
    message: failure.message,
    path: failure.path ?? null,
    ruleId: failure.ruleId ?? null,
  })), ...unaudited];
  // A scan with something to scan that ran no audit failed, and its `unaudited` failures say why.
  let completed = audits.every(audit => audit.result?.status === "completed")
    && audited
    && pathFailures.length === 0
    && !refRules?.skipped.some(failure => failure.code === "adapter-error");
  for (const skipped of (refRules === undefined ? catalog?.skipped.filter(policy => policy.path?.endsWith(".rerail")) : undefined) ?? []) {
    failures.push({ code: "adapter-error", message: skipped.reason, path: skipped.path, ruleId: null });
    completed = false;
  }
  if (rr.length > 0) {
    try {
      let configured: Parameters<typeof scanRrRules>[7];
      if (catalog !== undefined && configuredPolicies.length > 0) {
        const checkout = lookupCheckout(database, root)!;
        configured = {
          policies: new Map(configuredPolicies.map(policy => [policy.id, policy])),
          mode: resolveRrModes(database, catalog, checkout, parentCheckouts(database, checkout).map(parent => parent.checkoutPath), refRules !== undefined),
        };
      }
      let selected = configured === undefined ? rr : configuredPolicies.filter(configured.mode.enabled).map(policy => policy.path);
      if (explicitRr) selected = selected.filter(path => {
        try { loadedRr.load(resolve(path)); return true; }
        catch (error) {
          failures.push({ code: "adapter-error", path: resolve(path), ruleId: null, message: error instanceof Error ? error.message : String(error) });
          completed = false;
          return false;
        }
      });
      const evaluated = await scanRrRules(selected, root, analysisStore, "", analysisRun, paths, source, configured, refRules, loadedRr);
      failures.push(...evaluated.failures);
      if (evaluated.failures.length > 0) completed = false;
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
  for (const { path, message, pathIdentity } of source?.sourceFailures() ?? []) {
    failures.push({ code: "adapter-error", message, path, pathIdentity, ruleId: null });
    completed = false;
  }
  if (source !== undefined || options.scope === "git-diff") {
    for (const row of [...violations, ...failures]) {
      if (row.path !== null) row.pathIdentity ??= textPath(row.path);
    }
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
      ...checkoutRef(root),
    },
    ...(source === undefined || options.revision === undefined ? {} : { source: { revision: options.revision, tree: source.tree, commit: source.commit ?? null } }),
    scope: options.scope,
    ...target,
    rulesSource: !audited ? null : explicitRr ? { kind: "rr" } : refRules?.provenance ?? { kind: useHome ? "home" : "worktree" },
    packs: declaredPacks === undefined ? homePacks(rules) : [...declaredPacks.packs],
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
  const { root, requested } = scanRoot(options);
  // `--pack` replaces the declared packs: the file is not read, and the scan runs the home's installed version of each.
  const declared = options.rr?.length || options.packs.length > 0 ? undefined : await declaredPacksOf(root, options.rulesRef);
  const analysisScope: ScanReport["analysisScope"] = options.analysisScope === undefined
    ? { kind: "directory", path: relative(root, requested).split(sep).join("/") || "." }
    : { kind: "named", name: options.analysisScope };
  const cache: ScanReport["cache"] = { publication: "unavailable", collection: "skipped" };
  const durableStore = scanContentStore(homeDatabase);
  assertContentStoreOwner(durableStore);
  const packs = installedPackIds(options.packs);
  const source = options.revision === undefined ? undefined : new binding.GitRevisionSource(root, options.revision, options.analysisScope!, requested);
  let analysisRun: NativeAnalysisScopeRun | undefined;
  let cacheWarning: string | undefined;
  try {
    mkdirSync(dirname(homeDatabase), { recursive: true, mode: 0o700 });
    if (directory) {
      const registration = await acquireDirectoryAnalysisRegistration(homeDatabase, requested, durableStore);
      analysisRun = new binding.AnalysisScopeRun(durableStore, requested, true, scanId, registration.generation, analysisScopeHome(homeDatabase), registration.identity);
    } else {
      analysisRun = acquireNamedAnalysisRun(homeDatabase, options.analysisScope!, durableStore, scanId);
    }
  } catch (error) {
    cacheWarning = error instanceof Error ? error.message : String(error);
  }
  const analysisStore = analysisRun === undefined ? ":memory:" : durableStore;
  let report: ScanReport | undefined;
  try {
    analysisRun?.selectSource(root, source);
    if (source !== undefined)
      source.setSourceLanguages(JSON.stringify({ suffixes: Object.fromEntries(SOURCE_LANGUAGES), names: Object.fromEntries(SOURCE_NAMES) }));
    const home = openHomeDatabase(homeDatabase);
    let configuration: RerailDatabase | undefined;
    try {
      configuration = openRerailDatabase(":memory:");
      const database = configuration;
      const findings = await withDeclaredPacks(root, declared, options.packStore, declaredPacks =>
        scanReport(database, options, { root, home, packs, declaredPacks, scanId, startedAt, source, analysisStore, analysisRun }));
      report = { ...findings, analysisScope, cache };
    } finally {
      configuration?.$client.close();
      if (home?.$client.open) home.$client.close();
    }
    if (cacheWarning !== undefined) report.cacheWarnings = [cacheWarning];
    try {
      const warning = analysisRun?.finish(report.status === "completed");
      cache.publication = analysisRun?.publication ?? "unavailable";
      if (warning != null && cache.publication !== "released") report.cacheWarnings = [warning];
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
    if (analysisRun !== undefined && options.analysisScope !== undefined) try {
      finishNamedAnalysisRegistration(homeDatabase, options.analysisScope, scanId, analysisRun);
    } catch (error) {
      const message = `Could not finish named scope registration: ${String(error)}`;
      if (report !== undefined) (report.cacheWarnings ??= []).push(message);
      else process.stderr.write(`rerail: ${message}\n`);
    }
    if (analysisRun !== undefined) try {
      await binding.collectAnalysisCacheAsync(durableStore, [analysisRun.storeIdentity]);
      cache.collection = "completed";
    } catch (error) {
      cache.collection = "failed";
      const message = `Analysis cache collection failed: ${error instanceof Error ? error.message : String(error)}`;
      if (report !== undefined) (report.cacheWarnings ??= []).push(message);
      else process.stderr.write(`rerail: ${message}\n`);
    }
  }
}

/** One line per violation, then a summary: the text `rerail scan` prints without `--json`. */
export function describeScan(report: ScanReport): string {
  const lines = report.violations.map(violation => {
    if (violation.location) {
      const { repository, commit, field } = violation.location;
      return `commit ${commit} in ${repository}${field === null ? "" : ` (${field})`} [${violation.ruleId}] ${violation.message}`;
    }
    const at = violation.path === null ? "" : `${violation.path}${violation.start === null ? "" : `:${violation.start.line}:${violation.start.column}`} `;
    return `${at}[${violation.ruleId}] ${violation.message}`;
  });
  for (const failure of report.failures) lines.push(`failure ${failure.code}${failure.path === null ? "" : ` ${failure.path}`}: ${failure.message}`);
  if (report.cache?.publication === "released") lines.push("The analysis scope was released before publication.");
  if (report.cache?.publication === "unavailable") lines.push("Cache results could not be published.");
  for (const warning of report.cacheWarnings ?? []) lines.push(warning);
  const scope = report.scope === "git-diff" ? `git-diff${report.base === null ? "" : ` against ${report.base.ref}`}` : "repository";
  if (report.source !== undefined) {
    lines.push(`Source ${report.source.commit === null ? "tree " + report.source.tree : "commit " + report.source.commit}`);
  }
  lines.push(`Scan ${report.status}: ${report.repository.displayName} (${scope}), ${report.rules.length} rules, ${report.summary.violations} violations, ${report.summary.failures} failures`);
  return lines.join("\n");
}
