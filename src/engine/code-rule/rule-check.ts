/**
 * Check a code rule without storing it: compile it as the catalog does, run its cases, and run it over a checkout's
 * files the way a repository scan reads them, behind the same data boundary. Nothing is written to the catalog or the
 * content store. `rerail rules test` reports through it, and Merge Assist validates drafted and peer rules with it.
 */
import { existsSync, readFileSync, type Dirent } from "node:fs";
import { lstat, opendir, readFile, stat } from "node:fs/promises";
import { join, relative, resolve, sep } from "node:path";
import { z } from "zod";
import { deniedByPathBoundary } from "../path-boundary.js";
import { stableJsonText } from "../stable-json.js";
import {
  encodeCodeRuleWorkspaceRequest,
  loadNativeBinding,
  type NativeBinding,
  type NativeCompiledCodeRuleDsl,
  type NativeGitIgnoreMatcher,
} from "../native/native.js";
import type { CodeRuleSourceLanguage } from "./code-rule.js";
import { bundledCodeRuleDslDirectory, CodeRuleCompileError } from "./code-rule-compile.js";
import {
  IGNORED_DIRECTORIES,
  languageFor,
  sourceFactsCache,
  sourceFileFacts,
  type SourceFactsCache,
  type SourceFileFacts,
} from "./code-rule-enforcer.js";
import type { CodeRule } from "./code-rule-schema.js";

/** The same bounds a repository scan walks with. */
const REPOSITORY_MAX_FILES = 100_000;
const REPOSITORY_MAX_FILE_BYTES = 32_000_000;
const REPOSITORY_MAX_SOURCE_BYTES = 512 * 1024 * 1024;
const MATCHED_LINE_LENGTH = 200;
const DEFAULT_MAX_MATCHES = 20;

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

export interface RuleCheckMatch {
  readonly path: string;
  readonly line: number;
  readonly column: number;
  readonly message: string;
}

export interface RuleCheckResult {
  readonly valid: boolean;
  /** Why the rule could not be checked, for a person: a compile error, an Engine that cannot load, unreadable ignore rules. */
  readonly error: string | null;
  readonly cases: readonly RuleCheckCaseResult[];
  /** The first `maxMatches` matches, by path, line and column. */
  readonly matches: readonly RuleCheckMatch[];
  /** Every match. */
  readonly matchCount: number;
}

export interface RuleCheckOptions {
  /** The checkout's top folder: cases resolve imports against it and the repository pass reads below it. */
  readonly root: string;
  readonly cases: readonly RuleCheckCase[];
  /** Repository-relative paths the repository pass is limited to; every file below `root` when omitted. */
  readonly paths?: readonly string[];
  /** How many matches the result lists; 20 by default. */
  readonly maxMatches?: number;
  /** Whether the repository pass leaves out files Git ignores, as the repository's `followGitignore`; on by default. */
  readonly followGitignore?: boolean;
}

/** What the repository pass found, for a caller that reports more than {@link RuleCheckResult} carries. */
export interface RepositoryPassResult {
  /** Every match, by path, line and column. */
  readonly matches: readonly RuleCheckMatch[];
  /** Files some clause could not check (the first such failure's message each), and the messages of failures that name no file. */
  readonly unchecked: { readonly files: ReadonlyArray<{ path: string; message: string }>; readonly unattributed: readonly string[] };
  /** Files and folders the data boundary kept the pass from reading. A count only; their names stay out. */
  readonly leftOut: number;
  /** The text of a match's line, trimmed and shortened. */
  matchedLine(match: RuleCheckMatch): string;
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
/** The part of the Engine's evaluation a check reads: where each violation starts (zero-based), its message, and each failure. */
const evaluationSchema = z.object({
  evaluation: z.object({
    violations: z.array(z.object({
      path: z.string().optional(),
      message: z.string().optional(),
      span: z.object({ startLine: z.number().int(), startColumn: z.number().int() }).optional(),
    })),
    failures: z.array(z.object({ code: z.string(), message: z.string(), path: z.string().optional() })),
  }),
});
type Evaluation = z.infer<typeof evaluationSchema>["evaluation"];

/**
 * Violations and failures of `compiled` over `files`, in a private workspace with no content store attached. Import
 * targets resolve against the files below `root` the workspace does not hold, as in the daemon.
 */
async function evaluate({ binding, compiled }: CompiledCheck, root: string, files: readonly SourceFile[]): Promise<Evaluation> {
  const workspace = new binding.CodeRuleDslWorkspace();
  try {
    workspace.addCodeRule(compiled);
    workspace.setRoot(resolve(root));
    const accepted = files.filter(file => workspace.acceptsPath(file.path));
    if (accepted.length === 0) return { violations: [], failures: [] };
    const result = await workspace.applyBufferAsync(encodeCodeRuleWorkspaceRequest({ revision: 1, files: accepted, removed: [] }));
    return evaluationSchema.parse(JSON.parse(result)).evaluation;
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
  const language = languageFor(path);
  let caught = false;
  let note = "";
  if (language === undefined) note = " (not a file type rules read)";
  else if (!check.compiled.inScope(path)) note = " (outside the files this rule covers)";
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

/** One file a repository scan would read, with its size on disk, or undefined when the scan skips it. */
async function repositoryFile(
  compiled: NativeCompiledCodeRuleDsl,
  root: string,
  path: string,
  language: CodeRuleSourceLanguage,
  bytesSoFar: number,
  cache: SourceFactsCache,
): Promise<{ file: SourceFile; size: number } | undefined> {
  const absolute = join(root, path);
  const size = (await stat(absolute)).size;
  if (size > REPOSITORY_MAX_FILE_BYTES || bytesSoFar + size > REPOSITORY_MAX_SOURCE_BYTES) return undefined;
  const source = await readFile(absolute);
  if (source.includes(0)) return undefined;
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
  let ignored = (_path: string, _directory: boolean): boolean => false;
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

/** A repository-relative path made of plain segments: no absolute path, `.`, `..`, empty segment, backslash or NUL. */
function plainPath(path: string): boolean {
  if (path === "" || path.startsWith("/") || /[\\\0]/u.test(path)) return false;
  return path.split("/").every(segment => segment !== "" && segment !== "." && segment !== "..");
}

/**
 * The regular files among `paths` a scan would visit, in name order: a path that is missing, not plain, below an
 * ignored directory, a folder `prune` refuses or a symlinked folder, or that is itself not a regular file, is skipped.
 */
async function* selectedPaths(root: string, paths: readonly string[], prune: (path: string) => boolean): AsyncGenerator<string> {
  const folders = new Map<string, boolean>();
  const folderReadable = async (folder: string, name: string): Promise<boolean> => {
    const known = folders.get(folder);
    if (known !== undefined) return known;
    let readable = !IGNORED_DIRECTORIES.has(name);
    if (readable) {
      const entry = await lstat(join(root, folder)).catch(() => undefined);
      readable = entry !== undefined && entry.isDirectory() && !prune(folder);
    }
    folders.set(folder, readable);
    return readable;
  };
  for (const path of [...new Set(paths)].sort((left, right) => left.localeCompare(right))) {
    if (!plainPath(path)) continue;
    const segments = path.split("/");
    let readable = true;
    for (let index = 1; index < segments.length && readable; index += 1) {
      readable = await folderReadable(segments.slice(0, index).join("/"), segments[index - 1]!);
    }
    if (!readable) continue;
    const entry = await lstat(join(root, path)).catch(() => undefined);
    if (entry?.isFile() === true) yield path;
  }
}

/**
 * The files a repository scan would hand the rule (the same ignored directories, file types and bounds) that the
 * data boundary lets the pass read, and how many files and folders it left out.
 */
async function repositoryFiles(
  check: CompiledCheck,
  root: string,
  followGitignore: boolean,
  paths: readonly string[] | undefined,
): Promise<{ files: SourceFile[]; leftOut: number }> {
  const withheld = repositoryBoundary(check.binding, root, followGitignore);
  const files: SourceFile[] = [];
  const cache = sourceFactsCache();
  let bytes = 0;
  let leftOut = 0;
  const prune = (path: string): boolean => {
    if (!withheld(path, true)) return false;
    leftOut += 1;
    return true;
  };
  for await (const path of paths === undefined ? repositoryPaths(root, prune) : selectedPaths(root, paths, prune)) {
    if (files.length >= REPOSITORY_MAX_FILES) break;
    const language = languageFor(path);
    if (language === undefined || !check.compiled.inScope(path)) continue;
    if (withheld(path, false)) {
      leftOut += 1;
      continue;
    }
    const read = await repositoryFile(check.compiled, root, path, language, bytes, cache);
    if (read === undefined) continue;
    bytes += read.size;
    files.push(read.file);
  }
  return { files, leftOut };
}

/** Run the rule over `root`'s files, or only over `paths` among them, behind the data boundary. */
export async function repositoryPass(
  check: CompiledCheck,
  root: string,
  options: { followGitignore: boolean; paths?: readonly string[] },
): Promise<RepositoryPassResult> {
  const { files, leftOut } = await repositoryFiles(check, root, options.followGitignore, options.paths);
  const sources = new Map(files.map(file => [file.path, file.source]));
  const evaluation = await evaluate(check, root, files);
  const matches = evaluation.violations
    .map(violation => ({
      path: violation.path ?? "",
      line: (violation.span?.startLine ?? 0) + 1,
      column: (violation.span?.startColumn ?? 0) + 1,
      message: violation.message ?? "",
    }))
    .sort((left, right) => left.path.localeCompare(right.path) || left.line - right.line || left.column - right.column);
  const { files: uncheckedFiles, unattributed } = uncheckedPlaces(evaluation.failures);
  return {
    matches,
    unchecked: {
      files: [...uncheckedFiles].map(([path, message]) => ({ path, message })).sort((left, right) => left.path.localeCompare(right.path)),
      unattributed,
    },
    leftOut,
    matchedLine: match => (sources.get(match.path)?.split("\n")[match.line - 1] ?? "").trim().slice(0, MATCHED_LINE_LENGTH),
  };
}

/**
 * Compile `rule` and run its cases; then the repository pass over `root`, honouring `followGitignore` and the path
 * boundary as `rerail rules test` does, limited to `paths` when given. A rule that cannot be checked is reported
 * (`valid: false` with the reason), not thrown.
 */
export async function checkCodeRule(rule: CodeRule, options: RuleCheckOptions): Promise<RuleCheckResult> {
  try {
    const check = compileCheck(rule);
    const cases: RuleCheckCaseResult[] = [];
    for (const entry of options.cases) cases.push(await checkCase(check, options.root, entry));
    const pass = await repositoryPass(check, options.root, {
      followGitignore: options.followGitignore ?? true,
      ...(options.paths === undefined ? {} : { paths: options.paths }),
    });
    const maxMatches = Math.max(0, options.maxMatches ?? DEFAULT_MAX_MATCHES);
    return { valid: true, error: null, cases, matches: pass.matches.slice(0, maxMatches), matchCount: pass.matches.length };
  } catch (error) {
    if (!(error instanceof RuleCheckError)) throw error;
    return { valid: false, error: error.message, cases: [], matches: [], matchCount: 0 };
  }
}
