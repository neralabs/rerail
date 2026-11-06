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
import { existsSync, mkdirSync, mkdtempSync, readFileSync, realpathSync, rmSync, symlinkSync, truncateSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, relative, resolve, sep } from "node:path";
import { eq } from "drizzle-orm";
import type { AuditInput, AuditRecorder, AuditResult, AuditRuleProvenance } from "../engine/audit/audit.js";
import { CheckoutOwners } from "../engine/catalog/checkout-owner.js";
import { checkoutStatus, lookupCheckout, recordCheckout } from "../engine/catalog/repository-registry.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import { rulePackPackageOf } from "../engine/catalog/rule-packs.js";
import { setRuleSetting, setUpRepository } from "../engine/catalog/rule-configuration.js";
import { upsertLibraryRule } from "../engine/catalog/rule-library.js";
import type { RuleDocument } from "../engine/catalog/rule-store.js";
import { isWorktreePack, WORKTREE_PACK_ID, WORKTREE_RULES_FOLDER, worktreeRuleCatalog } from "../engine/catalog/worktree-pack.js";
import { CheckoutCodeRuleRouter } from "../engine/code-rule/checkout-code-rule-router.js";
import type { CodeRuleViolation } from "../engine/code-rule/code-rule.js";
import { CheckoutCatalogs, startingCheckout } from "../engine/code-rule/code-rule-dsl-catalog.js";
import { readGitBlobs, runGit } from "../engine/git.js";
import { defaultRerailDatabasePath, openRerailDatabase, openRerailDatabaseAsIs, type RerailDatabase } from "../engine/storage/database.js";
import { repositories } from "../engine/storage/schema.js";
import { setRepositoryStarted } from "../runtime/repository-start.js";

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
  source: { kind: string; id: string; packId: string | null; ref?: string };
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
  scope: "repository" | "git-diff";
  base: { ref: string; mergeBase: string } | null;
  paths: string[] | null;
  rulesSource: { kind: "home" } | { kind: "worktree" } | { kind: "ref"; ref: string };
  status: "completed" | "failed";
  rules: ScanRule[];
  violations: ScanViolation[];
  failures: ScanFailure[];
  summary: { violations: number; failures: number };
}

export interface HeadlessScanOptions {
  repo: string;
  scope: "repository" | "git-diff";
  base?: string;
  rulesRef?: string;
  packs: readonly string[];
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
  return result.status === 0 ? result.stdout.trim() : null;
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
  readonly documents: RuleDocument[];
  readonly skipped: ScanFailure[];
}

/**
 * The rule documents in `.rerail/rules/` at `ref`, loaded exactly as a checkout's own rules are
 * (`worktreeRuleCatalog`): the ref's files are written to a folder of their own and read from there, so a file that is
 * not a regular file, is too large, does not parse or compile, repeats an earlier file's id (the first by file name
 * keeps it) or takes the id of an installed pack's rule is left out with why, and the others load. A ref that is not a
 * commit throws: a typo must not pass as a scan with no rules.
 */
async function rulesAtRef(root: string, ref: string, scratch: string): Promise<RefRules> {
  const resolved = gitMaybe(root, ["rev-parse", "--verify", "--quiet", `${ref}^{commit}`]);
  if (resolved === null) throw new Error(`--rules-ref ${ref} is not a commit in ${root}`);
  if (runGit(root, ["cat-file", "-e", `${resolved}:${WORKTREE_RULES_FOLDER}`]).status !== 0) return { documents: [], skipped: [] };
  const copy = join(scratch, "rules-ref");
  const folder = join(copy, WORKTREE_RULES_FOLDER);
  mkdirSync(folder, { recursive: true });
  // `<mode> <type> <object>\t<name>`: regular files are written as they are, anything else as what it is, so the
  // loader leaves it out as it would leave it out of a checkout.
  const entries = nulList(git(root, ["ls-tree", "-z", `${resolved}:${WORKTREE_RULES_FOLDER}`])).flatMap(entry => {
    const match = /^(\d+) (\w+) ([0-9a-f]+)\t(.+)$/su.exec(entry);
    if (match === null) return [];
    const [, mode, type, object, name] = match as unknown as [string, string, string, string, string];
    return name.includes("/") || name === "." || name === ".." ? [] : [{ mode, type, object, name }];
  });
  const blobs = await readGitBlobs(root, entries.filter(entry => entry.type === "blob").map(entry => entry.object), { maxBlobBytes: MAX_REF_RULE_BYTES });
  for (const { mode, type, object, name } of entries) {
    const target = join(folder, name);
    const blob = blobs.get(object);
    if (type === "blob" && mode === "120000") symlinkSync(blob?.toString("utf8") ?? "missing", target);
    else if (type === "blob" && blob !== undefined) writeFileSync(target, blob);
    else if (type === "blob") {
      // Larger than the loader reads: a file of that size, which it leaves out as too large.
      writeFileSync(target, "");
      truncateSync(target, MAX_REF_RULE_BYTES + 1);
    } else mkdirSync(target);
  }
  const installed = loadRuleCatalog();
  const catalog = worktreeRuleCatalog(installed, copy);
  const pack = catalog.packs.get(WORKTREE_PACK_ID);
  const documents = pack === undefined || !isWorktreePack(pack)
    ? []
    : pack.rules.flatMap(rule => {
      const document = catalog.documents.get(rule.id);
      return document?.kind === "code" ? [document] : [];
    });
  const skipped = catalog.skipped.filter(entry => !installed.skipped.includes(entry)).map((entry): ScanFailure => ({
    code: "rule-left-out",
    message: `${entry.reason}; it does not run`,
    path: `${ref}:${relative(copy, entry.path).split(sep).join("/")}`,
    ruleId: null,
  }));
  return { documents, skipped };
}

/** Past the loader's own limit for a rule file (1 MiB), so a larger one is left out as it would be in a checkout. */
const MAX_REF_RULE_BYTES = 1024 * 1024;

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
  const status = checkoutStatus(home, checkout);
  return status.started && status.setUp;
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
  const columnsOf = (database: RerailDatabase, table: string): string[] =>
    (database.$client.prepare(`PRAGMA table_info(${table})`).all() as Array<{ name: string }>).map(column => column.name);
  scratch.$client.pragma("foreign_keys = OFF");
  try {
    scratch.$client.transaction(() => {
      for (const table of tables) {
        const theirs = new Set(columnsOf(home, table));
        const columns = columnsOf(scratch, table).filter(column => theirs.has(column));
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

function repositoryOf(databasePath: string, root: string): { key: string; displayName: string } {
  const database = openRerailDatabase(databasePath, { readonly: true });
  try {
    const checkout = lookupCheckout(database, root);
    if (checkout === undefined) throw new Error(`${root} is not recorded`);
    const row = database.select({ displayName: repositories.displayName }).from(repositories)
      .where(eq(repositories.repositoryKey, checkout.repositoryKey)).get();
    return { key: checkout.repositoryKey, displayName: row?.displayName ?? checkout.repositoryKey };
  } finally {
    database.$client.close();
  }
}

/**
 * `sha256(ruleId \0 path \0 matched text)`, the matched text with its whitespace collapsed, so a finding keeps its
 * fingerprint when lines move or are re-indented. Without a span, the rule's code stands in for the text.
 */
export function violationFingerprint(violation: Pick<CodeRuleViolation, "ruleId" | "path" | "span" | "code">, readSource: (path: string) => Buffer | undefined): string {
  let text = `code:${violation.code}`;
  if (violation.path !== undefined && violation.span !== undefined) {
    const source = readSource(violation.path);
    if (source !== undefined) text = source.subarray(violation.span.startByte, violation.span.endByte).toString("utf8").replace(/\s+/gu, " ").trim();
  }
  return createHash("sha256").update(`${violation.ruleId}\0${violation.path ?? ""}\0${text}`).digest("hex").slice(0, 32);
}

function ruleOf(provenance: AuditRuleProvenance, refRuleIds: ReadonlySet<string>, rulesRef: string | undefined): ScanRule {
  const placement = "placement" in provenance ? provenance.placement : [];
  const mode = (placement.find(entry => entry.folder === "") ?? placement[0])?.mode ?? "log";
  const source = "source" in provenance
    ? { kind: provenance.source.kind as string, id: provenance.source.id, packId: provenance.source.packId }
    : { kind: "unknown", id: provenance.ruleId, packId: null };
  const fromRef = rulesRef !== undefined && refRuleIds.has(provenance.ruleId) && source.kind === "library_rule";
  return {
    ruleId: provenance.ruleId,
    payloadHash: "payloadHash" in provenance ? provenance.payloadHash : "",
    mode,
    source: fromRef ? { kind: "ref", id: provenance.ruleId, packId: null, ref: rulesRef! } : source,
  };
}

/** Converts the engine's position to 1-based. The engine's lines and columns are 0-based (verified by test). */
function position(line: number, column: number): ScanPosition {
  return { line: line + 1, column: column + 1 };
}

/** The checkout `options.repo` is in, once the options that depend on the scope are checked. */
function scanRoot(options: HeadlessScanOptions): string {
  const requested = realpathSync(resolve(options.repo));
  const top = gitMaybe(requested, ["rev-parse", "--show-toplevel"]);
  if (top === null) throw new Error(`${requested} is not inside a Git checkout`);
  const root = realpathSync(top);
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

/** `run` with a scratch folder that holds the scan's own content store, restoring the environment and removing the folder after. */
async function withScanScratch<T>(home: RerailDatabase | undefined, run: (scratch: string) => Promise<T>): Promise<T> {
  const scratch = mkdtempSync(join(tmpdir(), "rerail-scan-"));
  const previousStore = process.env.RERAIL_CONTENT_STORE_PATH;
  // The machine-wide content store belongs to the daemon; this process keeps its own, thrown away after.
  if (previousStore === undefined || previousStore === "") process.env.RERAIL_CONTENT_STORE_PATH = join(scratch, "content.sqlite");
  try {
    return await run(scratch);
  } finally {
    if (home?.$client.open === true) home.$client.close();
    if (previousStore === undefined) delete process.env.RERAIL_CONTENT_STORE_PATH;
    else process.env.RERAIL_CONTENT_STORE_PATH = previousStore;
    rmSync(scratch, { recursive: true, force: true });
  }
}

/** What the scan covers: the paths asked for, what differs from the base or HEAD, or null for the whole repository. */
function scanTarget(options: HeadlessScanOptions, root: string): { base: ScanReport["base"]; paths: string[] | null } {
  if (options.paths !== undefined) {
    const paths = [...new Set(options.paths.map(path => path.replace(/^\.\//u, "")).filter(path => path.length > 0))]
      .filter(path => existsSync(join(root, path)))
      .sort((left, right) => left.localeCompare(right));
    return { base: null, paths };
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
async function runScanAudits(databasePath: string, root: string, sessionId: string, paths: readonly string[] | null): Promise<ScanAudit[]> {
  const audits: ScanAudit[] = [];
  const recorder: AuditRecorder = {
    start(input) { audits.push({ ...input, paths: [...input.paths] }); return String(audits.length - 1); },
    finish(auditId, result) { audits[Number(auditId)]!.result = result; },
    // Progress is not a result: an audit that never finishes stays `interrupted`, and the scan fails.
    progress(auditId, result) { audits[Number(auditId)]!.result = { status: "interrupted", ...result }; },
    interrupt() {},
  };
  const catalogs = new CheckoutCatalogs(databasePath);
  const owners = new CheckoutOwners();
  const router = new CheckoutCodeRuleRouter({
    sessionRoot: root,
    startingCheckout: startingCheckout(owners, root).checkoutRoot,
    owners,
    catalogs,
    steerDelivery: { requestSteer: async () => undefined },
    audits: recorder,
    followGitignore: catalogs.followsGitignore(root),
  });
  try {
    await router.openWorkspace(sessionId);
    if (paths === null) await router.audit(sessionId, { scope: "repository", mode: "log" });
    else if (paths.length > 0) await router.audit(sessionId, { scope: "touched-files", mode: "log", paths: [...paths] });
  } finally {
    await router.close();
    catalogs.close();
  }
  return audits;
}

function scanRules(audits: readonly ScanAudit[], refRuleIds: ReadonlySet<string>, rulesRef: string | undefined): Map<string, ScanRule> {
  const rules = new Map<string, ScanRule>();
  for (const audit of audits)
    for (const provenance of audit.ruleProvenance ?? []) if (!rules.has(provenance.ruleId)) rules.set(provenance.ruleId, ruleOf(provenance, refRuleIds, rulesRef));
  return rules;
}

function scanSourceReader(root: string): (path: string) => Buffer | undefined {
  const sources = new Map<string, Buffer | undefined>();
  return (path: string): Buffer | undefined => {
    if (!sources.has(path)) {
      try { sources.set(path, readFileSync(join(root, path))); } catch { sources.set(path, undefined); }
    }
    return sources.get(path);
  };
}

function scanViolations(audits: readonly ScanAudit[], rules: ReadonlyMap<string, ScanRule>, root: string): ScanViolation[] {
  const readSource = scanSourceReader(root);
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
    fingerprint: violationFingerprint(violation, readSource),
  }));
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

function scanRulesSource(rulesRef: string | undefined, useHome: boolean): ScanReport["rulesSource"] {
  if (rulesRef !== undefined) return { kind: "ref", ref: rulesRef };
  return useHome ? { kind: "home" } : { kind: "worktree" };
}

/** Runs the scan in its scratch folder and reports it: the scan's own database, always, with the home's only read and closed before the scan runs. */
async function scanReport(
  scratch: string, options: HeadlessScanOptions,
  { root, home, useHome, packs, scanId, startedAt }: { root: string; home: RerailDatabase | undefined; useHome: boolean; packs: readonly string[]; scanId: string; startedAt: number },
): Promise<ScanReport> {
  const databasePath = join(scratch, "rerail.sqlite");
  const refRules = options.rulesRef === undefined ? undefined : await rulesAtRef(root, options.rulesRef, scratch);
  let refRuleIds: Set<string>;
  try {
    refRuleIds = prepareScratchDatabase(databasePath, root, home, useHome, packs, refRules, options.ruleDocuments);
  } finally {
    home?.$client.close();
  }
  const repository = repositoryOf(databasePath, root);
  const { base, paths } = scanTarget(options, root);
  const audits = await runScanAudits(databasePath, root, `scan-${scanId}`, paths);
  const rules = scanRules(audits, refRuleIds, options.rulesRef);
  const violations = scanViolations(audits, rules, root);
  const failures = [...refRules?.skipped ?? [], ...audits.flatMap(audit => audit.result?.failures ?? []).map((failure): ScanFailure => ({
    code: failure.code,
    message: failure.message,
    path: failure.path ?? null,
    ruleId: failure.ruleId ?? null,
  }))];
  const completed = audits.every(audit => audit.result?.status === "completed") && (audits.length > 0 || paths?.length === 0);
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
    scope: options.scope,
    base,
    paths,
    rulesSource: scanRulesSource(options.rulesRef, useHome),
    status: completed ? "completed" : "failed",
    rules: [...rules.values()].sort((left, right) => left.ruleId.localeCompare(right.ruleId)),
    violations,
    failures,
    summary: { violations: violations.length, failures: failures.length },
  };
}

/** Runs one headless scan. Throws on bad input (not a checkout, unknown ref); an audit that fails is `status: "failed"`. */
export async function runHeadlessScan(options: HeadlessScanOptions): Promise<ScanReport> {
  const startedAt = Date.now();
  const scanId = randomUUID();
  const root = scanRoot(options);
  const homeDatabase = options.databasePath ?? defaultRerailDatabasePath();
  const packs = installedPackIds(options.packs);
  const home = openHomeDatabase(homeDatabase);
  const useHome = homeIsScanConfiguration(options, home, root, packs);
  return withScanScratch(home, scratch => scanReport(scratch, options, { root, home, useHome, packs, scanId, startedAt }));
}

/** One line per violation, then a summary: the text `rerail scan` prints without `--json`. */
export function describeScan(report: ScanReport): string {
  const lines = report.violations.map(violation => {
    const at = violation.path === null ? "" : `${violation.path}${violation.start === null ? "" : `:${violation.start.line}:${violation.start.column}`} `;
    return `${at}[${violation.ruleId}] ${violation.message}`;
  });
  for (const failure of report.failures) lines.push(`failure ${failure.code}${failure.path === null ? "" : ` ${failure.path}`}: ${failure.message}`);
  const scope = report.scope === "git-diff" ? `git-diff${report.base === null ? "" : ` against ${report.base.ref}`}` : "repository";
  lines.push(`Scan ${report.status}: ${report.repository.displayName} (${scope}), ${report.rules.length} rules, ${report.summary.violations} violations, ${report.summary.failures} failures`);
  return lines.join("\n");
}
