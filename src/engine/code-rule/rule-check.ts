/**
 * Check a code rule without storing it: compile it as the catalog does, run its cases, and run it over a checkout's
 * files the way a repository scan reads them, behind the same data boundary. Nothing is written to the catalog or the
 * content store. `rerail rules test` reports through it.
 */
import { existsSync, readFileSync, type Dirent } from "node:fs";
import { opendir, readFile, stat } from "node:fs/promises";
import { join, relative, resolve, sep } from "node:path";
import { z } from "zod";
import { deniedByPathBoundary } from "../path-boundary.js";
import { stableJsonText } from "../stable-json.js";
import {
  encodeCodeRuleWorkspaceRequest,
  loadNativeBinding,
  RESULT_PAGE_BYTES,
  type NativeBinding,
  type NativeCompiledCodeRuleDsl,
  type NativeGitIgnoreMatcher,
} from "../native/native.js";
import type { CodeRuleSourceLanguage } from "./code-rule.js";
import { bundledCodeRuleDslDirectory, CodeRuleCompileError } from "./code-rule-compile.js";
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
} from "./code-rule-enforcer.js";
import { UNLISTED_TEXT_LANGUAGE } from "./code-rule-languages.js";
import type { CodeRule } from "./code-rule-schema.js";

/** The batches an audit sends its files to the Engine in: at most this many files, or about this many bytes. */
const BATCH_FILES = 32;
const BATCH_BYTES = 1024 * 1024;
const MATCHED_LINE_LENGTH = 200;

/** A rule the Engine cannot compile, an Engine that cannot load, or a repository pass that cannot respect the data boundary. */
export class RuleCheckError extends Error {
  constructor(message: string) {
    super(message);
    this.name = "RuleCheckError";
  }
}

export interface RuleCheckCase {
  readonly expected: "caught" | "passed";
  readonly label: string;
  readonly path: string;
  readonly content: string;
}

export interface RuleCheckCaseResult {
  readonly label: string;
  readonly expected: "caught" | "passed";
  readonly caught: boolean;
  readonly ok: boolean;
  /** Why a clause of the rule could not check the case's file, which then decides nothing: never `ok`. */
  readonly notChecked?: string;
}

/** A place the rule matches: one-based line and column, neither for a match on the file's path. */
export interface RuleCheckMatch {
  readonly path: string;
  readonly line?: number;
  readonly column?: number;
  readonly message: string;
}

/** What the repository pass found. */
export interface RepositoryPassResult {
  /** Every match, by path, line and column. */
  readonly matches: readonly RuleCheckMatch[];
  /** Files some clause could not check (the first such failure's message each), and the messages of failures that name no file. */
  readonly unchecked: { readonly files: ReadonlyArray<{ path: string; message: string }>; readonly unattributed: readonly string[] };
  /** Files in the rule's scope the pass skipped as binary: they hold a NUL byte. */
  readonly binary: number;
  /** Files and folders the data boundary kept the pass from reading. A count only; their names stay out. */
  readonly leftOut: number;
  /** The text of a match's line, trimmed and shortened; undefined for a match on the file's path. */
  matchedLine(match: RuleCheckMatch): string | undefined;
}

/** A compiled rule with the Engine binding that evaluates it. */
export interface CompiledCheck {
  readonly ruleId: string;
  readonly binding: NativeBinding;
  readonly compiled: NativeCompiledCodeRuleDsl;
}

function errorMessage(error: unknown): string {
  return error instanceof Error ? error.message : String(error);
}

/** The refusal for a rule the Engine cannot check because it could not be loaded. */
export function engineUnavailableMessage(ruleId: string, reason: string): string {
  return `the Engine is unavailable, so rule ${ruleId} cannot be checked: ${reason}`;
}

/** Compile with the same source text and bundled capabilities the catalog's compile check uses, keeping the result to evaluate. */
export function compileCheck(rule: CodeRule): CompiledCheck {
  const sourceText = stableJsonText(rule);
  if (sourceText === undefined) throw new RuleCheckError(new CodeRuleCompileError(rule.id, "the document is not JSON representable").message);
  let binding: NativeBinding;
  let capabilities: string;
  try {
    binding = loadNativeBinding();
    capabilities = readFileSync(resolve(bundledCodeRuleDslDirectory(), "capabilities.json"), "utf8");
  } catch (error) {
    throw new RuleCheckError(engineUnavailableMessage(rule.id, errorMessage(error)));
  }
  try {
    return { ruleId: rule.id, binding, compiled: binding.CodeRuleDslRuntime.compile(sourceText, "json", capabilities) };
  } catch (error) {
    throw new RuleCheckError(new CodeRuleCompileError(rule.id, errorMessage(error)).message);
  }
}

type SourceFile = { path: string; language: string; source: string } & SourceFileFacts;
/**
 * The part of a page of the Engine's results a check reads: where each violation starts (zero-based), its message, and
 * each failure.
 */
const resultPageSchema = z.object({
  cursor: z.number().int().nonnegative(),
  done: z.boolean(),
  violations: z.array(z.object({
    path: z.string().optional(),
    message: z.string().optional(),
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
async function evaluate({ binding, compiled }: CompiledCheck, root: string, files: readonly SourceFile[]): Promise<Evaluation> {
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

/** One case: whether the rule catches its content at its path, and whether that is what the case expects. */
export async function checkCase(check: CompiledCheck, root: string, entry: RuleCheckCase): Promise<RuleCheckCaseResult> {
  const { expected, label, path, content } = entry;
  // An unlisted file is read as text, but only for a rule that reads text, as audits read it.
  const language = languageFor(path) ?? (check.compiled.readsText() ? UNLISTED_TEXT_LANGUAGE : undefined);
  let caught = false;
  let note = "";
  if (!check.compiled.inScope(path)) note = " (outside the files this rule covers)";
  else if (language === undefined) note = " (not read: no language lists this file, and this rule has no regex or file clause)";
  else {
    // The graph reads the sample in place of the repository's file at its path.
    const facts = await sourceFileFacts(root, path, language, undefined, check.compiled.readsServerGraph(), { path, source: content });
    const evaluation = await evaluate(check, root, [{ path, language, source: content, ...facts }]);
    caught = evaluation.violations.some(violation => violation.path === undefined || violation.path === path);
    // A violation stands whatever else failed; without one, a clause that could not check the file decides nothing.
    const unchecked = caught ? undefined : uncheckedPlaces(evaluation.failures);
    const notChecked = unchecked === undefined ? undefined : (unchecked.files.get(path) ?? unchecked.unattributed[0]);
    if (notChecked !== undefined) return { expected, caught, ok: false, label, notChecked };
  }
  return { expected, caught, ok: caught === (expected === "caught"), label: `${label}${note}` };
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

/**
 * The repository pass's data boundary: whether it must not read the file or folder at `path`, since the lines it
 * matches go back to the agent that ran it. A hidden or sensitive path is always withheld, as is a checkout of its own
 * nested in this one (its files answer to its own ignore rules and setting, which this pass does not read). A path
 * Git ignores is withheld unless a person turned the repository's `followGitignore` off; a tracked file never is.
 * Where the pass follows `.gitignore` in a Git checkout but Git's rules cannot be read, nothing can be told apart, so
 * the pass refuses rather than read what may be ignored.
 */
function repositoryBoundary(binding: NativeBinding, root: string, followGitignore: boolean): (path: string, directory: boolean) => boolean {
  let ignored: (path: string, directory: boolean) => boolean = () => false;
  if (followGitignore) {
    let matcher: NativeGitIgnoreMatcher | null;
    try {
      matcher = binding.openGitIgnoreMatcher(root);
    } catch (error) {
      throw new RuleCheckError(`Git's ignore rules for ${root} could not be read, so the repository pass was not run: ${errorMessage(error)}`);
    }
    if (matcher === null && existsSync(join(root, ".git"))) {
      throw new RuleCheckError(`Git's ignore rules for ${root} could not be read, so the repository pass was not run`);
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
 * The files a repository scan would hand the rule (the same ignored directories, file types and bounds) that the
 * data boundary lets the pass read, how many files and folders it left out, and how many in-scope files it skipped
 * as binary.
 */
async function repositoryFiles(
  check: CompiledCheck,
  root: string,
  followGitignore: boolean,
): Promise<{ files: SourceFile[]; leftOut: number; binary: number }> {
  const withheld = repositoryBoundary(check.binding, root, followGitignore);
  const readsText = check.compiled.readsText();
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
    if (language === undefined || !check.compiled.inScope(path)) continue;
    if (withheld(path, false)) {
      leftOut += 1;
      continue;
    }
    const read = await repositoryFile(check.compiled, root, path, language, bytes, cache);
    if (read === "binary") binary += 1;
    if (read === undefined || read === "binary") continue;
    bytes += read.size;
    files.push(read.file);
  }
  return { files, leftOut, binary };
}

/** Run the rule over `root`'s files behind the data boundary. */
export async function repositoryPass(
  check: CompiledCheck,
  root: string,
  options: { followGitignore: boolean },
): Promise<RepositoryPassResult> {
  const { files, leftOut, binary } = await repositoryFiles(check, root, options.followGitignore);
  const sources = new Map(files.map(file => [file.path, file.source]));
  const lines = new Map<string, string[]>();
  const evaluation = await evaluate(check, root, files);
  const matches: RuleCheckMatch[] = evaluation.violations
    .map(({ path = "", span, message = "" }) =>
      // A match on the file's path has no span: it is the whole file, with no line.
      (span === undefined ? { path, message } : { path, line: span.startLine + 1, column: span.startColumn + 1, message }))
    .sort((left, right) => left.path.localeCompare(right.path) || (left.line ?? 0) - (right.line ?? 0) || (left.column ?? 0) - (right.column ?? 0));
  const { files: uncheckedFiles, unattributed } = uncheckedPlaces(evaluation.failures);
  return {
    matches,
    unchecked: {
      files: [...uncheckedFiles].map(([path, message]) => ({ path, message })).sort((left, right) => left.path.localeCompare(right.path)),
      unattributed,
    },
    binary,
    leftOut,
    matchedLine: match => {
      if (match.line === undefined) return undefined;
      if (!lines.has(match.path)) lines.set(match.path, sources.get(match.path)?.split("\n") ?? []);
      return (lines.get(match.path)![match.line - 1] ?? "").trim().slice(0, MATCHED_LINE_LENGTH);
    },
  };
}
