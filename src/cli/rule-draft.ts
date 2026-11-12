import { existsSync, readFileSync, type Dirent } from "node:fs";
import { opendir, readFile, stat } from "node:fs/promises";
import { join, relative, resolve, sep } from "node:path";
import { resolveModelPath } from "../runtime/session-paths.js";
import { z } from "zod";
import { libraryUpsertBodySchema } from "../api/repository-contracts.js";
import { parseArguments } from "../daemon/daemon.js";
import { defaultRerailDatabasePath, openRerailDatabase, type RerailDatabase } from "../engine/storage/database.js";
import { loadRuleCatalog } from "../engine/catalog/rule-catalog.js";
import {
  AUDIT_MAX_FILES,
  AUDIT_MAX_SOURCE_BYTES,
  IGNORED_DIRECTORIES,
  MAX_SOURCE_BYTES,
  languageFor,
  sourceFactsCache,
  sourceFileFacts,
  type SourceFactsCache,
  type SourceFileFacts,
} from "../engine/code-rule/code-rule-enforcer.js";
import {
  encodeCodeRuleWorkspaceRequest,
  loadNativeBinding,
  RESULT_PAGE_BYTES,
  type NativeBinding,
  type NativeCompiledCodeRuleDsl,
  type NativeGitIgnoreMatcher,
} from "../engine/native/native.js";
import { nativeGitAccess } from "../engine/native/git-access.js";
import { bundledCodeRuleDslDirectory, CodeRuleCompileError, EngineUnavailableError } from "../engine/code-rule/code-rule-compile.js";
import { CODE_RULE_FORMAT, codeRuleSchema, type CodeRule } from "../engine/code-rule/code-rule-schema.js";
import { ConversationRuleRegistry } from "../engine/catalog/registry.js";
import { repositoryIdentityLocation, repositoryName } from "../engine/catalog/repository-identity.js";
import { normalizeFolder, type CheckoutContext, type EffectiveRule, type LibraryRule, type RuleCatalog } from "../engine/catalog/repository-model.js";
import type { RuleDocument } from "../engine/catalog/rule-store.js";
import { BLOCK_STEERS_NOTE, type RuleMode } from "../engine/catalog/rule-mode.js";
import { checkoutStatus, followsGitignore, lookupCheckout, recordCheckout } from "../engine/catalog/repository-registry.js";
import { addAttachment, listRepositoryRules, setUpRepository } from "../engine/catalog/rule-configuration.js";
import {
  copyRerailRuleToLibrary,
  getLibraryRule,
  libraryHoldsId,
  libraryRuleFolders,
  libraryRuleRepositories,
  rerailRuleCopiesIn,
  rerailRuleFolders,
  upsertLibraryRule,
  type CopiedRule,
} from "../engine/catalog/rule-library.js";
import { isApproved, ruleSubject } from "../engine/catalog/rule-approval.js";
import { parseConversationRule, sampleSourceSchema, SAMPLE_BOUNDARIES, type ConversationRule } from "../engine/catalog/rule-schema.js";
import { readRulesDashboard } from "../presentation/session-presentation.js";
import { deniedByPathBoundary } from "../engine/path-boundary.js";
import { stableJsonText } from "../engine/stable-json.js";
import type { CodeRuleSourceLanguage } from "../engine/code-rule/code-rule.js";
import { UNLISTED_TEXT_LANGUAGE } from "../engine/code-rule/code-rule-languages.js";

/** How many repository matches and unchecked files the text report shows; the JSON report lists every one. */
const REPOSITORY_SHOWN = 5;
/** The batches an audit sends its files to the Engine in: at most this many files, or about this many bytes. */
const BATCH_FILES = 32;
const BATCH_BYTES = 1024 * 1024;
const MATCH_TEXT_LENGTH = 200;
const CASE_LABEL_LENGTH = 60;

const codeRuleCaseSchema = z.strictObject({ path: z.string().min(1), content: z.string() });
const conversationCaseSchema = z.strictObject({
  source: sampleSourceSchema,
  boundary: z.enum(SAMPLE_BOUNDARIES),
  text: z.string(),
  input: z.string().optional(),
});
const draftCasesSchema = z.strictObject({
  caught: z.array(z.unknown()).default([]),
  passed: z.array(z.unknown()).default([]),
});

type CodeRuleCase = z.infer<typeof codeRuleCaseSchema>;
type ConversationCase = z.infer<typeof conversationCaseSchema>;
type Expectation = "caught" | "passed";

/**
 * A draft rule, the folder of its repository it is added at, and its cases. `folder` is undefined when the draft names
 * none: a new rule is added to the whole repository, and one already added stays where it is. `""` is the whole
 * repository.
 */
export type RuleDraft =
  | { kind: "code"; rule: CodeRule; folder: string | undefined; cases: ReadonlyArray<{ expected: Expectation; sample: CodeRuleCase }> }
  | { kind: "conversation"; rule: ConversationRule; folder: string | undefined; cases: ReadonlyArray<{ expected: Expectation; sample: ConversationCase }> };

/** The draft's `folder`, normalized; undefined when it names none. */
function draftFolder(folder: string | undefined): string | undefined {
  if (folder === undefined) return undefined;
  try {
    return normalizeFolder(folder);
  } catch (error) {
    throw new RuleDraftError(`folder: ${errorMessage(error)}`);
  }
}

/** A draft is added to the repository of the checkout it is added from; it never names another. */
const TARGET_FIELDS = ["repository", "checkout", "attach"] as const;

export interface RuleDraftCaseResult {
  expected: Expectation;
  caught: boolean;
  ok: boolean;
  label: string;
  /** Why a clause of the rule could not check the case's file, which then decides nothing: never `ok`. */
  notChecked?: string;
}

/** A place the rule matches: one-based line and column and the line's text, none of them for a match on the file's path. */
export interface RuleDraftRepositoryMatch {
  path: string;
  line?: number;
  column?: number;
  text?: string;
}

/** A file some clause of the rule could not check: the first such failure's message. */
export interface RuleDraftUncheckedFile {
  path: string;
  message: string;
}

export interface RuleDraftReport {
  ok: boolean;
  valid: boolean;
  error?: string;
  cases: RuleDraftCaseResult[];
  repository: {
    /** Every place the rule matches, in path, line and column order. */
    matches: RuleDraftRepositoryMatch[];
    /** Every file some clause could not check, and the messages of such failures that name no file. */
    unchecked: { files: RuleDraftUncheckedFile[]; unattributed: string[] };
    /** Files in the rule's scope the pass skipped as binary: they hold a NUL byte. */
    binary: number;
    /**
     * Files and folders the data boundary kept the pass from reading: hidden, sensitive by name, or ignored by Git
     * while the repository follows `.gitignore`. A count only; their names stay out of the report.
     */
    leftOut: number;
  } | null;
}

/** A draft the schema, the Engine, or a case rejects; `rules test` reports it rather than throwing. */
export class RuleDraftError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RuleDraftError";
  }
}

function issuesText(error: z.ZodError, prefix: readonly PropertyKey[] = []): string {
  return error.issues
    .map(issue => {
      const path = [...prefix, ...issue.path].map(String).join(".");
      return path.length === 0 ? issue.message : `${path}: ${issue.message}`;
    })
    .join("; ");
}

function parseCases<T>(schema: z.ZodType<T>, values: readonly unknown[], list: Expectation): Array<{ expected: Expectation; sample: T }> {
  return values.map((value, index) => {
    const parsed = schema.safeParse(value);
    if (!parsed.success) throw new RuleDraftError(issuesText(parsed.error, ["cases", list, index]));
    return { expected: list, sample: parsed.data };
  });
}

/** Validate a draft exactly as `PUT /api/library-rules/{id}` validates its body, plus the optional `cases`. */
export function parseRuleDraft(value: unknown): RuleDraft {
  const envelope = z.looseObject({ cases: z.unknown().optional() }).safeParse(value);
  if (!envelope.success) throw new RuleDraftError("the draft must be a JSON object");
  const { cases: rawCases, ...body } = envelope.data;
  const target = TARGET_FIELDS.find(field => field in body);
  if (target !== undefined) throw new RuleDraftError(`${target}: a draft is added to this checkout's repository and cannot name ${target}`);
  // The body's `rule` is a union of both kinds, whose failure names neither; a code draft is told what its rule lacks.
  if (body.kind === "code") {
    const rule = codeRuleSchema.safeParse(body.rule);
    if (!rule.success) throw new RuleDraftError(issuesText(rule.error, ["rule"]));
  }
  const upsert = libraryUpsertBodySchema.safeParse(body);
  if (!upsert.success) throw new RuleDraftError(issuesText(upsert.error));
  const folder = draftFolder(upsert.data.folder);
  const cases = draftCasesSchema.safeParse(rawCases ?? {});
  if (!cases.success) throw new RuleDraftError(issuesText(cases.error, ["cases"]));
  if (upsert.data.kind === "code") {
    const schema = codeRuleCaseSchema;
    return {
      kind: "code",
      rule: upsert.data.rule as CodeRule,
      folder,
      cases: [...parseCases(schema, cases.data.caught, "caught"), ...parseCases(schema, cases.data.passed, "passed")],
    };
  }
  let rule: ConversationRule;
  try {
    rule = parseConversationRule(upsert.data.rule);
  } catch (error) {
    throw new RuleDraftError(errorMessage(error));
  }
  return {
    kind: "conversation",
    rule,
    folder,
    cases: [...parseCases(conversationCaseSchema, cases.data.caught, "caught"), ...parseCases(conversationCaseSchema, cases.data.passed, "passed")],
  };
}

/**
 * A rule document on its own, as a worktree keeps one in `.rerail/rules`, as a draft with no cases: a JSON object that
 * names neither `kind` nor `rule`, a code rule when its `format` is the code rule format and else a conversation rule.
 */
function asDraft(value: unknown): unknown {
  if (typeof value !== "object" || value === null || Array.isArray(value) || "kind" in value || "rule" in value) return value;
  return { kind: "format" in value && value.format === CODE_RULE_FORMAT ? "code" : "conversation", rule: value };
}

/** The draft in `file`; with `bareRule`, a rule document on its own is read as a draft with no cases (`asDraft`). */
export function readRuleDraft(file: string, options: { readonly bareRule?: boolean } = {}): RuleDraft {
  const text = readFileSync(resolve(file), "utf8");
  let value: unknown;
  try {
    value = JSON.parse(text);
  } catch (error) {
    throw new RuleDraftError(`${file} is not JSON: ${errorMessage(error)}`);
  }
  return parseRuleDraft(options.bareRule === true ? asDraft(value) : value);
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

function engineUnavailable(ruleId: string, reason: string): RuleDraftError {
  return new RuleDraftError(`the Engine is unavailable, so rule ${ruleId} cannot be checked: ${reason}`);
}

/** Compile with the same source text and bundled capabilities the catalog's compile check uses, keeping the result to evaluate. */
function compileCodeRule(rule: CodeRule): { binding: NativeBinding; compiled: NativeCompiledCodeRuleDsl } {
  const sourceText = stableJsonText(rule);
  if (sourceText === undefined) throw new RuleDraftError(new CodeRuleCompileError(rule.id, "the document is not JSON representable").message);
  let binding: NativeBinding;
  let capabilities: string;
  try {
    binding = loadNativeBinding();
    capabilities = readFileSync(resolve(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");
  } catch (error) {
    throw engineUnavailable(rule.id, errorMessage(error));
  }
  try {
    return { binding, compiled: binding.CodeRuleDslRuntime.compile(sourceText, "json", capabilities) };
  } catch (error) {
    throw new RuleDraftError(new CodeRuleCompileError(rule.id, errorMessage(error)).message);
  }
}

type SourceFile = { path: string; language: string; source: string } & SourceFileFacts;
/** The part of a page of the Engine's results the report reads: where each violation starts (zero-based), and each failure. */
const resultPageSchema = z.object({
  cursor: z.number().int().nonnegative(),
  done: z.boolean(),
  violations: z.array(z.object({
    path: z.string().optional(),
    span: z.object({ startLine: z.number().int(), startColumn: z.number().int() }).optional(),
  })),
  failures: z.array(z.object({ code: z.string(), message: z.string(), path: z.string().optional() })),
});
type Evaluation = Pick<z.infer<typeof resultPageSchema>, "violations" | "failures">;

/**
 * Violations and failures of `compiled` over `files`, in a private workspace with no content store attached. Import
 * targets resolve against the files below `root` the workspace does not hold, as in the daemon. The files go in the
 * batches an audit sends, with cross-file clauses held until a last empty apply sees them all; the whole result is
 * then read a page at a time, as an audit reads it.
 */
async function evaluate(binding: NativeBinding, compiled: NativeCompiledCodeRuleDsl, root: string, files: readonly SourceFile[]): Promise<Evaluation> {
  const workspace = new binding.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(compiled);
    workspace.setRoot(resolve(root));
    const accepted = files.filter(file => workspace.acceptsPath(file.path));
    if (accepted.length === 0) return { violations: [], failures: [] };
    let revision = 0;
    let batch: SourceFile[] = [];
    let bytes = 0;
    for (const [index, file] of accepted.entries()) {
      batch.push(file);
      bytes += Buffer.byteLength(file.source);
      if (batch.length < BATCH_FILES && bytes < BATCH_BYTES && index < accepted.length - 1) continue;
      revision += 1;
      await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision, files: batch, removed: [], deferGlobal: true }));
      batch = [];
      bytes = 0;
    }
    revision += 1;
    await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision, files: [], removed: [], quiet: true }));
    const evaluation: Evaluation = { violations: [], failures: [] };
    let cursor = 0;
    while (true) {
      const page = resultPageSchema.parse(JSON.parse(await workspace.resultPageAsync(cursor, RESULT_PAGE_BYTES)));
      evaluation.violations.push(...page.violations);
      evaluation.failures.push(...page.failures);
      if (page.done) return evaluation;
      if (page.cursor <= cursor) throw new Error("the Engine's result pages did not advance");
      cursor = page.cursor;
    }
  } finally {
    workspace.close();
  }
}

/**
 * What some clause of the rule could not check: per path, the first such failure's message, and the distinct messages
 * of such failures that name no file (a cross-file query or a budget), which leave every file's verdict unknown. A
 * `source-partial` failure is neither: the clauses ran around syntax errors and what they found stands.
 */
function uncheckedPlaces(failures: Evaluation["failures"]): { files: Map<string, string>; unattributed: string[] } {
  const files = new Map<string, string>();
  const unattributed = new Set<string>();
  for (const failure of failures) {
    if (failure.code === "source-partial") continue;
    if (failure.path === undefined) unattributed.add(failure.message);
    else if (!files.has(failure.path)) files.set(failure.path, failure.message);
  }
  return { files, unattributed: [...unattributed] };
}

async function policyCaseResult(
  binding: NativeBinding,
  compiled: NativeCompiledCodeRuleDsl,
  root: string,
  { expected, sample }: { expected: Expectation; sample: CodeRuleCase },
): Promise<RuleDraftCaseResult> {
  // An unlisted file is read as text, but only for a rule that reads text, as audits read it.
  const language = languageFor(sample.path) ?? (compiled.readsText() ? UNLISTED_TEXT_LANGUAGE : undefined);
  let caught = false;
  let note = "";
  if (!compiled.inScope(sample.path)) note = " (outside the files this rule covers)";
  else if (language === undefined) note = " (not read: no language lists this file, and this rule has no regex or file clause)";
  else {
    // The graph reads the sample in place of the repository's file at its path.
    const facts = await sourceFileFacts(root, sample.path, language, undefined, compiled.readsServerGraph(), { path: sample.path, source: sample.content });
    const file = { path: sample.path, language, source: sample.content, ...facts };
    const evaluation = await evaluate(binding, compiled, root, [file]);
    caught = evaluation.violations.some(violation => violation.path === undefined || violation.path === sample.path);
    // A violation stands whatever else failed; without one, a clause that could not check the file decides nothing.
    const unchecked = caught ? undefined : uncheckedPlaces(evaluation.failures);
    const notChecked = unchecked === undefined ? undefined : (unchecked.files.get(sample.path) ?? unchecked.unattributed[0]);
    if (notChecked !== undefined) return { expected, caught, ok: false, label: sample.path, notChecked };
  }
  return { expected, caught, ok: caught === (expected === "caught"), label: `${sample.path}${note}` };
}

/**
 * One file a repository scan would read, with its size on disk; `"binary"` when it holds a NUL byte, and undefined
 * when it is over the byte bounds. Either way the scan skips it.
 */
async function repositoryFile(
  compiled: NativeCompiledCodeRuleDsl,
  root: string,
  path: string,
  language: CodeRuleSourceLanguage,
  bytesSoFar: number,
  cache: SourceFactsCache,
): Promise<{ file: SourceFile; size: number } | "binary" | undefined> {
  const absolute = join(root, path);
  const size = (await stat(absolute)).size;
  if (size > MAX_SOURCE_BYTES || bytesSoFar + size > AUDIT_MAX_SOURCE_BYTES) return undefined;
  const source = await readFile(absolute);
  if (source.includes(0)) return "binary";
  return {
    file: { path, language, source: source.toString("utf8"), ...(await sourceFileFacts(root, path, language, cache, compiled.readsServerGraph())) },
    size,
  };
}

async function sortedEntries(directory: string): Promise<Dirent[]> {
  const entries: Dirent[] = [];
  for await (const entry of await opendir(directory)) entries.push(entry);
  return entries.sort((left, right) => left.name.localeCompare(right.name));
}

/** Whether `root`'s repository follows `.gitignore`, read without recording anything: on where Rerail cannot tell. */
function repositoryFollowsGitignore(root: string, databasePath = defaultRerailDatabasePath()): boolean {
  if (!existsSync(databasePath)) return true;
  let database: RerailDatabase | undefined;
  try {
    database = openRerailDatabase(databasePath, { readonly: true });
    return followsGitignore(database, lookupCheckout(database, root));
  } catch {
    return true;
  } finally {
    database?.$client.close();
  }
}

/**
 * The repository pass's data boundary: whether it must not read the file or folder at `path`, since the lines it
 * matches go back to the agent that ran it. A hidden or sensitive path is always withheld, as is a checkout of its own
 * nested in this one (its files answer to its own ignore rules and setting, which this pass does not read). A path
 * Git ignores is withheld unless a person turned the repository's `followGitignore` off; a tracked file never is.
 * Where the pass follows `.gitignore` in a Git checkout but Git's rules cannot be read, nothing can be told apart, so
 * the pass refuses rather than read what may be ignored.
 */
function repositoryBoundary(binding: NativeBinding, root: string): (path: string, directory: boolean) => boolean {
  let ignored = (_path: string, _directory: boolean): boolean => false;
  if (repositoryFollowsGitignore(root)) {
    let matcher: NativeGitIgnoreMatcher | null;
    try {
      matcher = nativeGitAccess(binding).openGitIgnoreMatcher(root);
    } catch (error) {
      throw new RuleDraftError(`Git's ignore rules for ${root} could not be read, so the repository pass was not run: ${errorMessage(error)}`);
    }
    if (matcher === null && existsSync(join(root, ".git"))) {
      throw new RuleDraftError(`Git's ignore rules for ${root} could not be read, so the repository pass was not run`);
    }
    if (matcher !== null) {
      const opened = matcher;
      ignored = (path, directory) => {
        try {
          return opened.ignored(path, directory);
        } catch {
          return true;
        }
      };
    }
  }
  return (path, directory) =>
    deniedByPathBoundary(path) || (directory && existsSync(join(root, path, ".git"))) || ignored(path, directory);
}

/**
 * Every regular file under `root` a scan visits, as a `/`-separated path below it, depth-first in name order, skipping
 * ignored directories, symlinks and the folders `prune` refuses.
 */
async function* repositoryPaths(root: string, prune: (path: string) => boolean): AsyncGenerator<string> {
  const pending = [root];
  while (pending.length > 0) {
    const directory = pending.pop()!;
    for (const entry of await sortedEntries(directory)) {
      const absolute = join(directory, entry.name);
      const path = relative(root, absolute).split(sep).join("/");
      if (entry.isDirectory()) {
        if (!IGNORED_DIRECTORIES.has(entry.name) && !prune(path)) pending.push(absolute);
      } else if (entry.isFile()) yield path;
    }
  }
}

/**
 * The files a repository scan would hand `compiled` (the same ignored directories, file types and bounds) that the
 * data boundary lets the pass read, how many files and folders it left out, and how many in-scope files it skipped
 * as binary.
 */
async function repositoryFiles(
  binding: NativeBinding,
  compiled: NativeCompiledCodeRuleDsl,
  root: string,
): Promise<{ files: SourceFile[]; leftOut: number; binary: number }> {
  const withheld = repositoryBoundary(binding, root);
  const readsText = compiled.readsText();
  const files: SourceFile[] = [];
  const cache = sourceFactsCache();
  let bytes = 0;
  let leftOut = 0;
  let binary = 0;
  const prune = (path: string): boolean => {
    if (!withheld(path, true)) return false;
    leftOut += 1;
    return true;
  };
  for await (const path of repositoryPaths(root, prune)) {
    if (files.length >= AUDIT_MAX_FILES) break;
    // An unlisted file is read as text, but only for a rule that reads text, as audits read it.
    const language = languageFor(path) ?? (readsText ? UNLISTED_TEXT_LANGUAGE : undefined);
    if (language === undefined || !compiled.inScope(path)) continue;
    if (withheld(path, false)) {
      leftOut += 1;
      continue;
    }
    const read = await repositoryFile(compiled, root, path, language, bytes, cache);
    if (read === "binary") binary += 1;
    if (read === undefined || read === "binary") continue;
    bytes += read.size;
    files.push(read.file);
  }
  return { files, leftOut, binary };
}

async function repositoryPass(
  binding: NativeBinding,
  compiled: NativeCompiledCodeRuleDsl,
  root: string,
): Promise<NonNullable<RuleDraftReport["repository"]>> {
  const { files, leftOut, binary } = await repositoryFiles(binding, compiled, root);
  const sources = new Map(files.map(file => [file.path, file.source]));
  const lines = new Map<string, string[]>();
  const evaluation = await evaluate(binding, compiled, root, files);
  const matches: RuleDraftRepositoryMatch[] = evaluation.violations
    .map(({ path = "", span }) => {
      // A match on the file's path has no span: it is the whole file, with no line.
      if (span === undefined) return { path };
      if (!lines.has(path)) lines.set(path, sources.get(path)?.split("\n") ?? []);
      const text = (lines.get(path)![span.startLine] ?? "").trim().slice(0, MATCH_TEXT_LENGTH);
      return { path, line: span.startLine + 1, column: span.startColumn + 1, text };
    })
    .sort((left, right) => left.path.localeCompare(right.path) || (left.line ?? 0) - (right.line ?? 0) || (left.column ?? 0) - (right.column ?? 0));
  const { files: uncheckedFiles, unattributed } = uncheckedPlaces(evaluation.failures);
  const unchecked = [...uncheckedFiles]
    .map(([path, message]) => ({ path, message }))
    .sort((left, right) => left.path.localeCompare(right.path));
  return { matches, unchecked: { files: unchecked, unattributed }, binary, leftOut };
}

function conversationCaseResults(rule: ConversationRule, cases: ReadonlyArray<{ expected: Expectation; sample: ConversationCase }>): RuleDraftCaseResult[] {
  let registry: ConversationRuleRegistry;
  try {
    registry = new ConversationRuleRegistry({
      modelPath: resolveModelPath(process.cwd()),
    });
  } catch (error) {
    throw engineUnavailable(rule.id, errorMessage(error));
  }
  try {
    registry.replace([rule]);
  } catch (error) {
    throw new RuleDraftError(errorMessage(error));
  }
  return cases.map(({ expected, sample }) => {
    const caught = registry.match(sample, { respectRepeat: false }).some(match => match.ruleId === rule.id);
    const text = sample.text.length > CASE_LABEL_LENGTH ? `${sample.text.slice(0, CASE_LABEL_LENGTH - 1)}…` : sample.text;
    return { expected, caught, ok: caught === (expected === "caught"), label: `${sample.source}: ${text}` };
  });
}

async function policyCaseResults(
  binding: NativeBinding,
  compiled: NativeCompiledCodeRuleDsl,
  root: string,
  cases: ReadonlyArray<{ expected: Expectation; sample: CodeRuleCase }>,
): Promise<RuleDraftCaseResult[]> {
  const results: RuleDraftCaseResult[] = [];
  for (const entry of cases) results.push(await policyCaseResult(binding, compiled, root, entry));
  return results;
}

/**
 * Run a draft's cases as `rules test` runs them, with import targets resolved against `root`, and skip its
 * repository pass. A draft the Engine refuses throws `RuleDraftError`.
 */
export async function testRuleDraftCases(draft: RuleDraft, root: string): Promise<RuleDraftCaseResult[]> {
  if (draft.kind === "conversation") return conversationCaseResults(draft.rule, draft.cases);
  const { binding, compiled } = compileCodeRule(draft.rule);
  return policyCaseResults(binding, compiled, root, draft.cases);
}

/**
 * Try a draft without storing it: validate it as storing would, run its cases, and for a policy rule count its
 * matches across `repo`'s files. Nothing is written to the catalog or the content store.
 */
export async function testRuleDraft(draft: RuleDraft, repo: string): Promise<RuleDraftReport> {
  if (draft.kind === "conversation") {
    const cases = conversationCaseResults(draft.rule, draft.cases);
    return { ok: cases.every(result => result.ok), valid: true, cases, repository: null };
  }
  const { binding, compiled } = compileCodeRule(draft.rule);
  const root = repositoryIdentityLocation(repo).workspaceRoot;
  const cases = await policyCaseResults(binding, compiled, root, draft.cases);
  const repository = await repositoryPass(binding, compiled, root);
  return { ok: cases.every(result => result.ok), valid: true, cases, repository };
}

/**
 * `testRuleDraft` for a draft file, or a rule document on its own (a worktree's `.rerail/rules/<name>.json`), which has
 * no cases; an unreadable or invalid draft is reported, not thrown.
 */
export async function testRuleDraftFile(file: string, repo: string): Promise<RuleDraftReport> {
  try {
    return await testRuleDraft(readRuleDraft(file, { bareRule: true }), repo);
  } catch (error) {
    if (!(error instanceof RuleDraftError)) throw error;
    return { ok: false, valid: false, error: error.message, cases: [], repository: null };
  }
}

export interface AddedRule {
  ruleId: string;
  repositoryName: string;
  /** The repository folders the rule was added at; `""` is the whole repository. */
  folders: readonly string[];
  /** The By Rerail rule the draft edited, stored as `ruleId`, your copy of it; null for a rule of your own. */
  copiedFromRuleId: string | null;
  /** Whether adding the rule set the repository up, with this rule as its only one. */
  setUp: boolean;
  /** The mode the rule's content sets; `block` steers until block enforcement lands. */
  mode: RuleMode;
  /** Whether the rule's content is approved, so it steers; otherwise it runs in Log until `rerail rules approve`. */
  approved: boolean;
}

function libraryWriteError(error: unknown, ruleId: string): RuleDraftError {
  if (error instanceof EngineUnavailableError) return new RuleDraftError(`the Engine is unavailable, so rule ${ruleId} cannot be checked`);
  return new RuleDraftError(errorMessage(error));
}

/** Why the By Rerail rule `id` has no copy to make in the repository `name`. */
function notAddedText(id: string, name: string): string {
  return `${id} is not added to ${name}, so there is nothing here for your copy to replace`;
}

/** Names the copies of `id` the repository `name` already has: the first is the one to change. */
function copyAddedText(id: string, name: string, copies: readonly string[]): string {
  return `${name} already has your copy of ${id}, ${copies.join(", ")}. Change that one: rerail rules show ${copies[0]}`;
}

/** The repository `name`, or a folder of it. */
function placeText(name: string, folder: string): string {
  return folder === "" ? name : `${name}/${folder}`;
}

/**
 * Store a draft of a By Rerail rule as your copy, as `library_rules_upsert` does for its id: added wherever the
 * original is in the checkout's repository, where approving the copy switches the original off. Refused where the
 * original is not added, or where a copy of it already is: that copy is the one to change.
 */
function addRerailRuleCopy(database: RerailDatabase, ruleCatalog: RuleCatalog, checkout: CheckoutContext, draft: RuleDraft): AddedRule {
  const sourceId = draft.rule.id;
  const name = repositoryName(checkout.checkoutPath);
  if (draft.folder !== undefined) {
    throw new RuleDraftError(`folder: your copy of ${sourceId} is added where ${sourceId} is, so the draft cannot name a folder`);
  }
  const copies = rerailRuleCopiesIn(database, checkout.repositoryKey, sourceId);
  if (copies.length > 0) throw new RuleDraftError(copyAddedText(sourceId, name, copies));
  if (rerailRuleFolders(database, ruleCatalog, checkout.repositoryKey, sourceId).length === 0) {
    throw new RuleDraftError(notAddedText(sourceId, name));
  }
  let copied: CopiedRule;
  try {
    copied = copyRerailRuleToLibrary(database, ruleCatalog, checkout.repositoryKey, sourceId, draft.rule);
  } catch (error) {
    throw libraryWriteError(error, sourceId);
  }
  const ruleId = copied.rule.document.rule.id;
  // A copy that was approved, removed and made again with the same content is approved already.
  const approved = isApproved(database, ruleSubject(ruleId, copied.rule.payloadHash));
  return { ruleId, repositoryName: name, folders: copied.folders, copiedFromRuleId: sourceId, setUp: false, mode: draft.rule.mode, approved };
}

/**
 * Refuse to add your copy of a By Rerail rule, made for another repository, to one where the original runs and the
 * copy is not added: both would steer, and approving the copy, approved already, would not turn the original off.
 */
function refuseCopyFromElsewhere(database: RerailDatabase, ruleCatalog: RuleCatalog, checkout: CheckoutContext, id: string): void {
  const copiedFrom = getLibraryRule(database, id)?.copiedFromRuleId ?? null;
  if (copiedFrom === null || libraryRuleFolders(database, checkout.repositoryKey, id).length > 0) return;
  if (rerailRuleFolders(database, ruleCatalog, checkout.repositoryKey, copiedFrom).length === 0) return;
  const name = repositoryName(checkout.checkoutPath);
  throw new RuleDraftError(`${id} is your copy of ${copiedFrom} for another repository, and ${copiedFrom} runs in ${name}, so both would steer here. To change ${copiedFrom} here: rerail rules show ${copiedFrom}`);
}

/**
 * Add the stored library rule of `draft` to the checkout's repository at the draft's folder, setting the repository up
 * with it if it is not yet. A rule already added there stays where it is when the draft names no folder, as a draft
 * from `rules show` does: changing a rule never widens where it runs. A draft that names `""` adds it to the whole
 * repository.
 */
function attachLibraryRule(database: RerailDatabase, checkout: CheckoutContext, draft: RuleDraft): { folders: string[]; setUp: boolean } {
  const attached = libraryRuleFolders(database, checkout.repositoryKey, draft.rule.id);
  if (draft.folder === undefined && attached.length > 0) return { folders: attached, setUp: false };
  const folder = draft.folder ?? "";
  const attachment = { sourceKind: "library_rule" as const, sourceId: draft.rule.id, folder };
  const setUp = checkoutStatus(database, checkout).setUp === false;
  if (setUp) setUpRepository(database, checkout.repositoryKey, { packs: [], rules: [attachment] });
  else addAttachment(database, { repositoryKey: checkout.repositoryKey, ...attachment });
  return { folders: [folder], setUp };
}

/**
 * Store the draft's rule in your library, through the same validation as `library_rules_upsert`, and add it to
 * `repo`'s repository ({@link attachLibraryRule}). A By Rerail rule is never replaced: a draft with its id is stored
 * as your copy ({@link addRerailRuleCopy}).
 */
export function addRuleDraft(draft: RuleDraft, repo: string, databasePath = defaultRerailDatabasePath()): AddedRule {
  const database = openRerailDatabase(databasePath);
  try {
    const checkout = recordCheckout(database, repo);
    const ruleCatalog = loadRuleCatalog();
    // A library rule a later release bundled the id of wins over the By Rerail rule, so a draft with its id edits it.
    if (ruleCatalog.documents.has(draft.rule.id) && !libraryHoldsId(database, draft.rule.id)) {
      return addRerailRuleCopy(database, ruleCatalog, checkout, draft);
    }
    refuseCopyFromElsewhere(database, ruleCatalog, checkout, draft.rule.id);
    try {
      upsertLibraryRule(database, ruleCatalog, draft.rule, draft.kind);
    } catch (error) {
      throw libraryWriteError(error, draft.rule.id);
    }
    const { folders, setUp } = attachLibraryRule(database, checkout, draft);
    const stored = getLibraryRule(database, draft.rule.id);
    const approved = stored !== undefined && isApproved(database, ruleSubject(draft.rule.id, stored.payloadHash));
    return { ruleId: draft.rule.id, repositoryName: repositoryName(checkout.checkoutPath), folders, copiedFromRuleId: null, setUp, mode: draft.rule.mode, approved };
  } finally {
    database.$client.close();
  }
}

/** A rule as a draft (`{ kind, rule }`), with what the person should know about changing it. */
export interface ShownRule {
  readonly draft: RuleDocument;
  readonly notes: readonly string[];
}

/** Where saving your rule `own` changes it, seen from the repository `name` (`key`, undefined when not recorded). */
function ownRuleNote(database: RerailDatabase, own: LibraryRule, name: string, key: string | undefined): string {
  const id = own.document.rule.id;
  const using = libraryRuleRepositories(database, id);
  const elsewhere = using.filter(repository => repository.repositoryKey !== key).map(repository => repository.name);
  if (elsewhere.length === using.length) {
    const original = own.copiedFromRuleId;
    return original === null
      ? `${id} is yours and not added to ${name}: rules add adds it to the whole repository, or to the draft's folder.`
      : `${id} is your copy of ${original} for another repository, and is not added to ${name}. To change ${original} here: rerail rules show ${original}`;
  }
  const also = elsewhere.length === 0 ? "" : ` It is also added to ${elsewhere.join(", ")}, where saving changes it too.`;
  return `${id} is yours, added to ${name}; saving changes it where it is.${also}`;
}

/**
 * The rule as it runs in the repository, which a copy must keep: the option values its settings give it and its mode
 * there (unless that is Log only while its pack waits for approval). Its shipped document when those values no longer
 * apply.
 */
function runningDocument(running: EffectiveRule, bundled: RuleDocument): RuleDocument {
  if (running.configError !== null) return bundled;
  const mode = running.decidedBy.mode.by === "awaiting-approval" ? bundled.rule.mode : running.mode;
  return { ...running.document, rule: { ...running.document.rule, mode } } as RuleDocument;
}

/** What saving an edit of the By Rerail rule `id` does in the repository `name`, given where it runs there. */
function rerailRuleNote(id: string, name: string, presences: readonly EffectiveRule[]): string {
  const off = presences.filter(rule => !rule.on).map(rule => placeText(name, rule.attachmentFolder));
  if (off.length > 0) {
    return `${id} is off in ${off.join(", ")}, and your copy would run there, on. Turn ${id} on first if it should run there.`;
  }
  return `${id} ships with Rerail. rules add stores your edit of it as your copy, added where ${id} is; approving the copy turns ${id} off there.`;
}

/** The By Rerail rule `id` as it runs in `repo`'s repository, with what saving an edit of it there does. */
function rerailRuleShown(database: RerailDatabase, ruleCatalog: RuleCatalog, bundled: RuleDocument, repo: string, id: string): ShownRule {
  const checkout = lookupCheckout(database, repo);
  if (checkout === undefined) return { draft: bundled, notes: [notAddedText(id, repositoryName(repo))] };
  const name = repositoryName(checkout.checkoutPath);
  const copies = rerailRuleCopiesIn(database, checkout.repositoryKey, id);
  if (copies.length > 0) return { draft: bundled, notes: [copyAddedText(id, name, copies)] };
  // The repository's settings only, not this checkout's: the copy runs in the whole repository, as its settings do.
  const presences = listRepositoryRules(database, ruleCatalog, checkout.repositoryKey).rules
    .filter(rule => rule.document.rule.id === id && rule.source.kind !== "library_rule");
  const running = presences[0];
  if (running === undefined) return { draft: bundled, notes: [notAddedText(id, name)] };
  return { draft: runningDocument(running, bundled), notes: [rerailRuleNote(id, name, presences)] };
}

/**
 * The rule `id` as a draft for `rules test` and `rules add`: one of yours, or a By Rerail rule as it runs in `repo`'s
 * repository, which `rules add` stores as your copy. Reading records nothing.
 */
export function showRule(id: string, repo: string, databasePath = defaultRerailDatabasePath()): ShownRule {
  const database = openRerailDatabase(databasePath);
  try {
    const own = getLibraryRule(database, id);
    if (own !== undefined) {
      const checkout = lookupCheckout(database, repo);
      return { draft: own.document, notes: [ownRuleNote(database, own, repositoryName(checkout?.checkoutPath ?? repo), checkout?.repositoryKey)] };
    }
    if (libraryHoldsId(database, id)) throw new RuleDraftError(`your rule ${id} could not be read; the dashboard shows why`);
    const ruleCatalog = loadRuleCatalog();
    const bundled = ruleCatalog.documents.get(id);
    if (bundled === undefined) {
      throw new RuleDraftError(`there is no rule ${id}: it is neither yours nor in an installed pack. A worktree's own rules are files in its .rerail/rules/`);
    }
    return rerailRuleShown(database, ruleCatalog, bundled, repo, id);
  } finally {
    database.$client.close();
  }
}

function caseLine(result: RuleDraftCaseResult): string {
  if (result.notChecked !== undefined) return `  ✗ not checked (${result.notChecked}), expected ${result.expected}: ${result.label}`;
  const outcome = result.caught ? "caught" : "passed";
  return result.ok
    ? `  ✓ ${outcome}, as expected: ${result.label}`
    : `  ✗ ${outcome}, expected ${result.expected}: ${result.label}`;
}

function repositoryLines(repository: RuleDraftReport["repository"]): string[] {
  if (repository === null) return ["  Repo: not checked for conversation rules."];
  const { matches, unchecked, binary, leftOut } = repository;
  const lines = [`  Repo: ${matches.length} ${matches.length === 1 ? "place matches" : "places match"}.`];
  for (const match of matches.slice(0, REPOSITORY_SHOWN)) {
    lines.push(match.line === undefined ? `    ${match.path}  (the whole file: no line)` : `    ${match.path}:${match.line}:${match.column}  ${match.text}`);
  }
  if (matches.length > REPOSITORY_SHOWN) lines.push(`    … and ${matches.length - REPOSITORY_SHOWN} more.`);
  const { files } = unchecked;
  if (files.length > 0) {
    lines.push(`  ${files.length} ${files.length === 1 ? "file was" : "files were"} not checked, so the rule may miss places there:`);
    for (const file of files.slice(0, REPOSITORY_SHOWN)) lines.push(`    ${file.path}  ${file.message}`);
    if (files.length > REPOSITORY_SHOWN) lines.push(`    … and ${files.length - REPOSITORY_SHOWN} more.`);
  }
  for (const message of unchecked.unattributed) lines.push(`  Not checked in any one file (${message}), so the rule may miss places anywhere.`);
  if (binary > 0) {
    lines.push(`  ${binary} ${binary === 1 ? "file was" : "files were"} skipped as binary: no rule reads a file holding a NUL byte.`);
  }
  if (leftOut > 0) {
    lines.push(`  ${leftOut} ${leftOut === 1 ? "file or folder was" : "files or folders were"} left out unread: hidden, sensitive by name, or ignored by Git.`);
  }
  return lines;
}

function verdictLine(report: RuleDraftReport): string {
  const { cases } = report;
  if (cases.length === 0) {
    return `● No cases to run${report.repository === null ? "" : "; only the repo was checked"}. A draft's caught and passed cases prove the rule.`;
  }
  const failed = cases.filter(result => !result.ok).length;
  if (failed === 0) return "● Every case matched.";
  return `● ${failed} ${failed === 1 ? "case did" : "cases did"} not match.`;
}

export function formatRuleDraftReport(report: RuleDraftReport): string {
  if (!report.valid) return `● The rule is not valid: ${report.error ?? "unknown error"}`;
  return [
    "● The rule is valid.",
    ...report.cases.map(caseLine),
    ...repositoryLines(report.repository),
    verdictLine(report),
  ].join("\n");
}

export interface RuleDraftCommandIo {
  stdout?: { write(text: string): unknown };
  stderr?: { write(text: string): unknown };
}

/** A small valid code-rule draft, the example `--help` prints. */
const EXAMPLE_DRAFT = {
  kind: "code",
  rule: {
    format: "rerail.policy/v1",
    id: "acme:no-console-log",
    revision: 1,
    mode: "steer",
    description: "Use the logger, not console.log.",
    scope: { include: ["src/**/*.ts"], exclude: [] },
    clauses: [{
      id: "no-console-log",
      forbid: { regex: { source: "file.content", pattern: String.raw`console\.log` } },
      violation: { anchor: "witness.span", code: "NO_CONSOLE_LOG", message: "Use the logger, not console.log." },
    }],
  },
  cases: {
    caught: [{ path: "src/a.ts", content: "console.log(1);\n" }],
    passed: [{ path: "src/b.ts", content: "logger.info(1);\n" }],
  },
};

/** What `rerail rules test|add|show --help` print: the commands, the draft format, and an example. */
function ruleDraftHelp(): string {
  return [
    "Usage:",
    "  rerail rules show <rule id> [--repo <dir>]",
    "  rerail rules test <draft.json | rule.json> [--repo <dir>] [--json]",
    "  rerail rules add <draft.json> [--repo <dir>]",
    "",
    "rules show prints a rule as a draft: one of yours, or a By Rerail rule from an installed pack.",
    "rules test checks a draft without storing it: it validates the rule, runs its cases, and for a code rule lists the",
    "places it matches in the repository. It also takes a rule document on its own (a worktree's .rerail/rules/<name>.json),",
    "which has no cases. rules add stores the rule in your library and adds it to the repository. A draft that keeps a",
    "By Rerail rule's id is stored as your copy of it, added where the original is; approving the copy turns the",
    "original off there.",
    "",
    "A draft is one JSON file:",
    "  {",
    '    "kind": "code" | "conversation",',
    '    "rule": { ... },',
    '    "folder"?: "<repo folder>",',
    '    "cases": {',
    '      "caught": [{ "path", "content" }],',
    '      "passed": [...]',
    "    }",
    "  }",
    "",
    "  kind    code for a code rule (a rerail.policy/v1 document), conversation for a conversation rule.",
    "  rule    the rule, as your library stores it.",
    "  folder  optional: the repository folder the rule is added at, \"\" for the whole repository. Absent, a new rule is",
    "          added to the whole repository and one already added stays where it is. A copy of a By Rerail rule",
    "          takes none: it is added where the original is.",
    "  cases   optional: the files the rule must catch and the files it must pass, each { \"path\", \"content\" }.",
    "          A conversation rule's cases are { \"source\", \"boundary\", \"text\", \"input\"? } instead: source is",
    `          user-input, assistant-text or tool:<name>; boundary is ${SAMPLE_BOUNDARIES.join(", ")}; input is a tool`,
    "          call's input as JSON text.",
    "",
    "Options:",
    "  --repo <dir>  the checkout whose repository the rule is shown for, tested against or added to (default: the current",
    "                directory)",
    "  --json        rules test only: print the report as JSON, with every match and every unchecked file",
    "",
    "Example: a code rule that forbids console.log in src/**/*.ts, with one case of each kind.",
    JSON.stringify(EXAMPLE_DRAFT, null, 2),
  ].join("\n");
}

type RuleDraftAction = "show" | "test" | "add";
type CommandOutput = NonNullable<RuleDraftCommandIo["stdout"]>;

/** The command's operand (the rule id for `show`, else the draft file), the checkout it acts for, and `--json`. */
function draftArguments(action: RuleDraftAction, args: readonly string[], usage: () => string): { operand: string; repo: string; json: boolean } {
  const operand = args[0];
  if (operand === undefined || operand.startsWith("--")) {
    const needs = action === "show" ? "a rule id" : "a draft file";
    throw new Error(`rules ${action} needs ${needs}; rerail rules ${action} --help shows its format\n\n${usage()}`);
  }
  const parsed = parseArguments(
    args.slice(1),
    { flags: action === "test" ? ["--json"] : [], options: ["--repo"] },
    argument => `Unknown rules ${action} option: ${argument}\n\n${usage()}`,
  );
  return { operand, repo: resolve(parsed.options.get("--repo") ?? process.cwd()), json: parsed.flags.has("--json") };
}

/** Where an added rule went: the repository, or its folders. */
function placesText(added: AddedRule): string {
  return added.folders.map(folder => placeText(added.repositoryName, folder)).join(", ");
}

function runShow(id: string, repo: string, stdout: CommandOutput, stderr: CommandOutput): number {
  let shown: ShownRule;
  try {
    shown = showRule(id, repo);
  } catch (error) {
    if (!(error instanceof RuleDraftError)) throw error;
    stderr.write(`● ${error.message}\n`);
    return 1;
  }
  // The draft alone on stdout, so it can be saved as a file; what the person should know goes to stderr.
  stdout.write(`${JSON.stringify(shown.draft, null, 2)}\n`);
  for (const note of shown.notes) stderr.write(`● ${note}\n`);
  return 0;
}

function addedLines(added: AddedRule): string[] {
  const source = added.copiedFromRuleId;
  const lines = [source === null
    ? `● Added ${added.ruleId} to ${placesText(added)}.`
    : `● Saved your copy of ${source} as ${added.ruleId}, added to ${placesText(added)} where ${source} is.`];
  if (added.setUp) lines.push(`  ${added.repositoryName} was not set up, so it is now set up with this rule.`);
  if (added.mode === "block") lines.push(`  ${added.ruleId} is on Block. ${BLOCK_STEERS_NOTE}`);
  if (added.approved) return lines;
  lines.push(source === null
    ? `  It runs in Log until a person approves it: rerail rules approve ${added.ruleId}`
    : `  It runs in Log beside ${source} until a person approves it, which turns ${source} off there: rerail rules approve ${added.ruleId}`);
  return lines;
}

async function runAdd(file: string, repo: string, stdout: CommandOutput, stderr: CommandOutput): Promise<number> {
  let added: AddedRule;
  try {
    added = addRuleDraft(readRuleDraft(file), repo);
  } catch (error) {
    if (!(error instanceof RuleDraftError)) throw error;
    stderr.write(`● The rule was not added: ${error.message}\n`);
    return 1;
  }
  for (const line of addedLines(added)) stdout.write(`${line}\n`);
  // The same dashboard the footer and steers link to, scoped to this checkout's repository when the daemon knows it.
  const dashboard = await readRulesDashboard({ cwd: repo });
  if (dashboard.url !== null) {
    const link = new URL(dashboard.url);
    link.hash = `rule/${encodeURIComponent(added.ruleId)}`;
    stdout.write(`  ${link.href}\n`);
  }
  return 0;
}

async function runTest(file: string, repo: string, json: boolean, stdout: CommandOutput): Promise<number> {
  const report = await testRuleDraftFile(file, repo);
  stdout.write(`${json ? JSON.stringify(report) : formatRuleDraftReport(report)}\n`);
  return report.ok ? 0 : 1;
}

/**
 * `rerail rules show <id>`, `rerail rules test <draft.json | rule.json>` and `rerail rules add <draft.json>`; with
 * `--help`, the draft format.
 */
export async function runRuleDraftCommand(
  action: RuleDraftAction,
  args: readonly string[],
  usage: () => string,
  io: RuleDraftCommandIo = {},
): Promise<number> {
  const stdout = io.stdout ?? process.stdout;
  const stderr = io.stderr ?? process.stderr;
  if (args.includes("--help") || args.includes("-h")) {
    stdout.write(`${ruleDraftHelp()}\n`);
    return 0;
  }
  const { operand, repo, json } = draftArguments(action, args, usage);
  if (action === "show") return runShow(operand, repo, stdout, stderr);
  if (action === "add") return await runAdd(operand, repo, stdout, stderr);
  return await runTest(operand, repo, json, stdout);
}
