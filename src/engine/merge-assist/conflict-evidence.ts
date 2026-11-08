/**
 * What the conflict policy measures on each side of a conflict, read from Git alone: the files and source lines the
 * side changes from where it left the base, the exported declarations it removes or changes in a way existing callers
 * notice (with those callers, read at the base), the reserved classes its change falls in, its diff, and the
 * repository's docs at the base. Read-only: every call goes through `runGit` or `readGitBlobs`.
 */
import { readGitBlobs, runGit } from "../git.js";
import { mergeBase } from "./branch-git.js";
import { type Declaration, declarations, shortName, type SourceLanguage, sourceLanguageOf } from "./alignment/symbols.js";
import { distinctiveSymbol, isDesignDoc, isFormatPath, isTestOrBenchPath, signatureKey } from "./alignment/units.js";
import type { ContractBreak, ReservedClass } from "./policy.js";

const LIST_OPTIONS = { maxBuffer: 64 * 1024 * 1024, timeout: 60_000 };
const MAX_SOURCE_BYTES = 512 * 1024;

/** Lockfiles and snapshots: generated, so no measure of the work. */
const GENERATED_PATH = /(?:^|\/)(?:package-lock\.json|npm-shrinkwrap\.json|pnpm-lock\.yaml|yarn\.lock|bun\.lockb?|Cargo\.lock|go\.sum|poetry\.lock|uv\.lock|Pipfile\.lock|Gemfile\.lock|composer\.lock)$|(?:^|\/)__snapshots__\/|\.snap$/u;
/** Folders whose declarations nothing outside the package imports. */
const PRIVATE_FOLDER = /(?:^|\/)(?:internal|_internal|tests?|testing|testdata|examples?|benches|benchmarks?|fixtures?|scripts|tools|xtask|cmd|docs?)\//iu;
/** Paths whose code guards access, secrets or untrusted input. */
const SECURITY_PATH = /(?:^|[/_.-])(?:auth\w*|security|secur\w*|crypto\w*|tls|ssl|certs?|certificates?|permissions?|csrf|cors|sanitiz\w*|secrets?|passwords?|tokens?|oauth\w*|jwt|acl|sandbox\w*)(?:[/_.-]|$)/iu;
/** Files a project announces changes in: changelogs, news, release notes, changeset and towncrier fragments. */
const CHANGELOG_PATH = /(?:^|\/)(?:change[_-]?log|changes|history|news|release[_-]?notes)[^/]*$|(?:^|\/)(?:\.changeset|changelog|changes|newsfragments|changelog\.d)\//iu;
/** Wording that declares a break. */
const BREAK_WORDING = /\bbreaking\b|\bBREAKING[ -]CHANGE\b|\bbackwards?[- ]incompatib\w*|\bnot\s+backwards?[- ]compatible\b|\b(?:remov(?:e|es|ed|al)|renam(?:e|es|ed))\s+(?:the\s+)?(?:deprecated|public)\b|^\s*[\w-]+(?:\([^)]*\))?!:/imu;
/** A towncrier-style fragment's type: `123.breaking.rst`, `456.removal.md`. */
const BREAK_FRAGMENT = /(?:^|\/)[^/]*\.(?:breaking|removal|deprecation)\.[a-z]+$/iu;

export interface ChangedFile {
  readonly path: string;
  /** The path at the base for a renamed file; the same path otherwise. */
  readonly oldPath: string;
  readonly status: "added" | "modified" | "deleted" | "renamed";
  readonly added: number;
  readonly removed: number;
}

export interface SideChanges {
  /** Where the side left the base: their merge base, or the base itself when they share none. */
  readonly from: string;
  readonly files: readonly ChangedFile[];
  /** Lines added and removed in source files that are not tests, benchmarks or generated. */
  readonly sourceLines: number;
  readonly sourceFiles: number;
}

/** Source code a caller depends on: a language Merge Assist reads, not a test, benchmark or generated file. */
export function isSourcePath(path: string): boolean {
  return sourceLanguageOf(path) !== null && !isTestOrBenchPath(path) && !GENERATED_PATH.test(path);
}

function revisionArgument(revision: string): boolean {
  return revision.length > 0 && !revision.startsWith("-") && !/[\0\n]/u.test(revision);
}

/** The files `head` changes from where it left `base`, with line counts; null when either commit is unknown. */
export function sideChanges(root: string, base: string, head: string): SideChanges | null {
  if (!revisionArgument(base) || !revisionArgument(head)) return null;
  const from = mergeBase(root, base, head) ?? base;
  const numstat = runGit(root, ["diff", "--numstat", "-z", "-M", "--no-ext-diff", "--end-of-options", from, head], LIST_OPTIONS);
  const status = runGit(root, ["diff", "--name-status", "-z", "-M", "--no-ext-diff", "--end-of-options", from, head], LIST_OPTIONS);
  if (numstat.status !== 0 || status.status !== 0) return null;
  const statuses = new Map<string, ChangedFile["status"]>();
  const statusFields = status.stdout.split("\0");
  for (let index = 0; index < statusFields.length - 1;) {
    const code = statusFields[index]!;
    if (code.startsWith("R") || code.startsWith("C")) {
      statuses.set(statusFields[index + 2]!, "renamed");
      index += 3;
    } else {
      statuses.set(statusFields[index + 1]!, code === "A" ? "added" : code === "D" ? "deleted" : "modified");
      index += 2;
    }
  }
  const files: ChangedFile[] = [];
  const fields = numstat.stdout.split("\0");
  for (let index = 0; index < fields.length - 1;) {
    const [added = "-", removed = "-", path = ""] = fields[index]!.split("\t");
    let oldPath = path;
    let newPath = path;
    if (path === "") {
      oldPath = fields[index + 1]!;
      newPath = fields[index + 2]!;
      index += 3;
    } else index += 1;
    files.push({ path: newPath, oldPath, status: statuses.get(newPath) ?? "modified", added: Number(added) || 0, removed: Number(removed) || 0 });
  }
  const source = files.filter(file => isSourcePath(file.path));
  return { from, files, sourceLines: source.reduce((sum, file) => sum + file.added + file.removed, 0), sourceFiles: source.length };
}

/** Whether `declaration` in `path` is part of the package's public surface. */
function publicDeclaration(path: string, language: SourceLanguage, declaration: Declaration): boolean {
  if (PRIVATE_FOLDER.test(path) || isTestOrBenchPath(path)) return false;
  if (language === "rust") return /^pub\s/u.test(declaration.signature);
  if (language === "python") return declaration.name.split(".").every(part => !part.startsWith("_") || part === "__init__");
  return declaration.exported;
}

/** The parameter list and what follows it (return type) of a function head, split at top-level commas. */
function parameters(signature: string): { prefix: string; params: string[]; suffix: string } | null {
  const open = signature.indexOf("(", signature.search(/\b(?:def|fn|function|func)\b/u) + 1);
  if (open === -1) return null;
  let depth = 0;
  let current = "";
  const params: string[] = [];
  for (let index = open; index < signature.length; index += 1) {
    const char = signature[index]!;
    if ("([{<".includes(char)) {
      depth += 1;
      if (depth === 1) continue;
    } else if (")]}>".includes(char) && signature[index - 1] !== "=") {
      depth -= 1;
      if (depth === 0) {
        if (current.trim() !== "") params.push(current.trim());
        return { prefix: signature.slice(0, open), params, suffix: signature.slice(index + 1) };
      }
    } else if (char === "," && depth === 1) {
      params.push(current.trim());
      current = "";
      continue;
    }
    current += char;
  }
  return null;
}

/** A parameter a caller may leave out: a default (`=` that is no comparison or arrow), an optional marker, or a rest parameter. */
function optionalParameter(language: SourceLanguage, parameter: string): boolean {
  if (language !== "python" && language !== "typescript" && language !== "javascript") return false;
  return /(?<![=!<>])=(?![=>])/u.test(parameter) || /^[\w$]+\?/u.test(parameter) || /^(?:\*|\.\.\.)/u.test(parameter);
}

/** The identifiers a declaration names: a type that keeps all of them and adds more only grew. */
function namesIn(text: string): Set<string> {
  return new Set(text.match(/[A-Za-z_$][\w$]*/gu) ?? []);
}

/**
 * Whether code written against `before` still works against `after`: a function that only gained optional parameters
 * (Python defaults, TypeScript optional and defaulted ones), a type or class that kept every name it had (fields,
 * variants, members, bases) and only gained more, a constant whose value but not type changed.
 */
export function compatibleChange(language: SourceLanguage, before: Declaration, after: Declaration): boolean {
  if (signatureKey(before.signature) === signatureKey(after.signature)) return true;
  if (before.kind === "constant") return signatureKey(before.signature.split("=")[0]!) === signatureKey(after.signature.split("=")[0]!);
  if (before.kind === "function" || before.kind === "method") {
    // A Go method's receiver is not a parameter.
    const receiverless = (signature: string) => (language === "go" ? signature.replace(/^func\s*\([^)]*\)\s*/u, "func ") : signature);
    const old = parameters(receiverless(before.signature));
    const next = parameters(receiverless(after.signature));
    if (old === null || next === null) return false;
    if (signatureKey(old.prefix) !== signatureKey(next.prefix) || signatureKey(old.suffix) !== signatureKey(next.suffix)) return false;
    if (next.params.length < old.params.length) return false;
    if (old.params.some((parameter, index) => signatureKey(parameter) !== signatureKey(next.params[index]!))) return false;
    return next.params.slice(old.params.length).every(parameter => optionalParameter(language, parameter));
  }
  if (before.kind === "type" || before.kind === "class") {
    const kept = namesIn(after.signature);
    return [...namesIn(before.signature)].every(name => kept.has(name));
  }
  return false;
}

/** Files at `revision`, other than `path`, that write `name` as a word: at most `limit`. */
function dependentsOf(root: string, revision: string, name: string, path: string, limit: number): string[] {
  const result = runGit(root, ["grep", "-l", "-w", "-F", "-I", "-e", name, revision, "--", ".", `:(exclude)${path}`], LIST_OPTIONS);
  if (result.status !== 0) return [];
  const prefix = `${revision}:`;
  return result.stdout.split("\n").filter(Boolean).map(line => (line.startsWith(prefix) ? line.slice(prefix.length) : line)).filter(file => !GENERATED_PATH.test(file)).slice(0, limit);
}

/** The most dependents recorded per break, and the most declarations a side's breaks are looked up for. */
const MAX_DEPENDENTS = 50;
const MAX_BREAKS = 60;

export interface ContractOptions {
  /** What the side says about itself (its sanitised description); a break it calls breaking is declared. */
  readonly intent: string;
  /** Names the policy declares broken (`contracts.declared`). */
  readonly declared: readonly string[];
}

/**
 * The exported declarations `changes` removes or changes in a way existing callers notice, each with the files at the
 * base that refer to it, whether it is public, and whether the break is declared: by the policy, by the side's
 * description, or by what the side adds to a changelog (breaking wording, a breaking or removal fragment, or the name).
 * A declaration removed from one changed file and declared in another moved; it did not break.
 */
export async function contractBreaks(root: string, head: string, changes: SideChanges, options: ContractOptions): Promise<ContractBreak[]> {
  const source = changes.files.filter(file => sourceLanguageOf(file.oldPath) !== null && !isTestOrBenchPath(file.oldPath) && file.status !== "added");
  if (source.length === 0) return [];
  const [before, after] = await Promise.all([
    readGitBlobs(root, source.map(file => `${changes.from}:${file.oldPath}`), { maxBlobBytes: MAX_SOURCE_BYTES, timeout: 60_000 }),
    readGitBlobs(root, changes.files.filter(file => file.status !== "deleted").map(file => `${head}:${file.path}`), { maxBlobBytes: MAX_SOURCE_BYTES, timeout: 60_000 }),
  ]);
  const afterByName = new Map<string, Declaration[]>();
  for (const file of changes.files) {
    const language = sourceLanguageOf(file.path);
    const text = after.get(`${head}:${file.path}`);
    if (language === null || text === undefined) continue;
    for (const declaration of declarations(language, text.toString("utf8"))) afterByName.set(declaration.name, [...(afterByName.get(declaration.name) ?? []), declaration]);
  }
  const declaring = await declaringText(root, head, changes, options.intent);
  const found: Array<Omit<ContractBreak, "dependents"> & { kind: Declaration["kind"] }> = [];
  for (const file of source) {
    const language = sourceLanguageOf(file.oldPath)!;
    const text = before.get(`${changes.from}:${file.oldPath}`);
    if (text === undefined) continue;
    for (const declaration of declarations(language, text.toString("utf8"))) {
      if (declaration.kind === "module" || !(declaration.exported || (language === "python" && shortName(declaration.name) === "__init__"))) continue;
      const successors = afterByName.get(declaration.name) ?? [];
      if (successors.some(successor => compatibleChange(language, declaration, successor))) continue;
      const successor = successors[0];
      const name = declaration.name;
      found.push({
        path: file.oldPath,
        name,
        kind: declaration.kind,
        change: successor === undefined ? "removed" : "changed",
        before: declaration.signature,
        after: successor?.signature ?? null,
        public: publicDeclaration(file.oldPath, language, declaration),
        declared: options.declared.includes(name) || options.declared.includes(shortName(name)) || BREAK_WORDING.test(declaring.wording) || new RegExp(`\\b${escapeRegExp(shortName(name))}\\b`, "u").test(declaring.changelog),
      });
    }
  }
  return found.slice(0, MAX_BREAKS).map(({ kind, ...change }) => {
    const short = shortName(change.name);
    const searchable = kind === "method" ? distinctiveSymbol(change.name) : short.length >= 3;
    return { ...change, dependents: searchable ? dependentsOf(root, changes.from, short, change.path, MAX_DEPENDENTS) : [] };
  });
}

function escapeRegExp(text: string): string {
  return text.replace(/[.*+?^${}()|[\]\\]/gu, "\\$&");
}

/** What can declare a break: the side's description and changelog additions (`wording`), and the changelog additions alone. */
async function declaringText(root: string, head: string, changes: SideChanges, intent: string): Promise<{ wording: string; changelog: string }> {
  const logs = changes.files.filter(file => CHANGELOG_PATH.test(file.path) && file.status !== "deleted");
  if (logs.length === 0) return { wording: intent, changelog: "" };
  const diff = runGit(root, ["diff", "--no-color", "--no-ext-diff", "-U0", "--end-of-options", changes.from, head, "--", ...logs.map(file => file.path)], LIST_OPTIONS);
  const added = diff.status === 0 ? diff.stdout.split("\n").filter(line => line.startsWith("+") && !line.startsWith("+++")).map(line => line.slice(1)).join("\n") : "";
  const fragments = logs.some(file => BREAK_FRAGMENT.test(file.path)) ? "\nbreaking" : "";
  return { wording: `${intent}\n${added}${fragments}`, changelog: added };
}

/**
 * Whether the side declares a breaking change: in its description, or in what it adds to a changelog (breaking wording,
 * a breaking or removal fragment). A declared break changes public behaviour even when no signature changes.
 */
export async function declaresBreak(root: string, head: string, changes: SideChanges, intent: string): Promise<boolean> {
  return BREAK_WORDING.test((await declaringText(root, head, changes, intent)).wording);
}

/**
 * The reserved classes a side's change falls in: the public API (a public declaration it breaks, or a break it
 * declares), security-sensitive code, and the data model.
 */
export function reservedClassesOf(changes: SideChanges, breaks: readonly ContractBreak[], declared: boolean): ReservedClass[] {
  const classes: ReservedClass[] = [];
  if (declared || breaks.some(change => change.public)) classes.push("public-api");
  const code = changes.files.filter(file => !isTestOrBenchPath(file.path));
  if (code.some(file => isSourcePath(file.path) && SECURITY_PATH.test(file.path))) classes.push("security");
  // A source file named for schemas (`to-json-schema.ts`) is code about schemas, not the data model; a migration is.
  if (code.some(file => isFormatPath(file.path) && (sourceLanguageOf(file.path) === null || /(?:^|\/)migrations?\//iu.test(file.path)))) classes.push("data-model");
  return classes;
}

/** A file's place in a diff shown to a reader: the code first, then tests, then everything else. */
function diffRank(path: string): number {
  if (isSourcePath(path)) return 0;
  if (sourceLanguageOf(path) !== null) return 1;
  return GENERATED_PATH.test(path) ? 3 : 2;
}

/**
 * The side's diff from where it left the base, for a reader: source first, then tests, then the rest, lockfiles and
 * snapshots left out; each file's part at most `maxFileChars`, the whole at most `maxChars`, what was cut named at the end.
 */
export function sideDiff(root: string, head: string, changes: SideChanges, maxChars: number, maxFileChars = 8_000): string {
  const result = runGit(root, ["diff", "--no-color", "--no-ext-diff", "-M", "-U3", "--end-of-options", changes.from, head], LIST_OPTIONS);
  if (result.status !== 0) return "";
  const parts = result.stdout.split(/^(?=diff --git )/mu).filter(part => part.startsWith("diff --git "));
  const pathOf = (part: string) => /^diff --git a\/.* b\/(.*)$/mu.exec(part)?.[1] ?? "";
  const ordered = parts.map(part => ({ path: pathOf(part), part })).filter(entry => !GENERATED_PATH.test(entry.path)).sort((left, right) => diffRank(left.path) - diffRank(right.path));
  const shown: string[] = [];
  const cut: string[] = [];
  let used = 0;
  for (const { path, part } of ordered) {
    const text = part.length <= maxFileChars ? part : `${part.slice(0, maxFileChars)}\n[cut: ${part.length - maxFileChars} more characters of ${path}]\n`;
    if (used + text.length > maxChars) {
      cut.push(path);
      continue;
    }
    shown.push(text);
    used += text.length;
  }
  return `${shown.join("")}${cut.length > 0 ? `\n[left out for length: ${cut.join(", ")}]\n` : ""}`;
}

export interface BaseDoc {
  readonly path: string;
  readonly text: string;
}

const TOP_LEVEL_DOC = /^(?:README|CONTRIBUTING|ARCHITECTURE|DESIGN|AGENTS|CLAUDE|HACKING|DEVELOPMENT|STYLE|CONVENTIONS)[^/]*\.(?:md|mdx|markdown|rst|txt|adoc)$/iu;
const MAX_TOP_LEVEL_CHARS = 5_000;
const EXCERPT_RADIUS = 1_200;
const MAX_EXCERPTS = 3;
const MAX_DOC_CANDIDATES = 600;
const DECISION_RULES = /^\.rerail\/rules\/[^/]+\.json$/u;

/** Terms a doc must name to be about the conflict: the anchors' file stems and symbols, distinctive ones only. */
function docTerms(anchors: { files: readonly string[]; symbols: readonly string[] }): string[] {
  const stems = anchors.files.map(file => file.split("/").pop()!.replace(/\.[^.]+$/u, "")).filter(stem => stem.length >= 5 && !/^(?:index|mod|lib|main|utils?|types?|init|__init__|README|CHANGELOG)$/iu.test(stem));
  return [...new Set([...stems, ...anchors.symbols.map(symbol => shortName(symbol)).filter(distinctiveSymbol)])].slice(0, 40);
}

/** Up to `MAX_EXCERPTS` windows of `text` around where it names a term, merged where they overlap. */
function excerpts(text: string, terms: readonly string[]): string {
  const hits: number[] = [];
  for (const term of terms) {
    let at = text.indexOf(term);
    while (at !== -1 && hits.length < 40) {
      hits.push(at);
      at = text.indexOf(term, at + term.length);
    }
  }
  hits.sort((left, right) => left - right);
  const windows: Array<[number, number]> = [];
  for (const hit of hits) {
    const start = Math.max(0, hit - EXCERPT_RADIUS);
    const end = Math.min(text.length, hit + EXCERPT_RADIUS);
    const last = windows.at(-1);
    if (last !== undefined && start <= last[1]) last[1] = Math.max(last[1], end);
    else windows.push([start, end]);
  }
  return windows.slice(0, MAX_EXCERPTS).map(([start, end]) => `${start > 0 ? "…" : ""}${text.slice(start, end)}${end < text.length ? "…" : ""}`).join("\n\n");
}

/**
 * The repository's docs at the base, as a reviewer of the conflict needs them, within `maxChars`: the decisions in force
 * (`.rerail/rules/*.json`, their descriptions), the top-level guides (README, CONTRIBUTING, ARCHITECTURE, AGENTS…,
 * cut), and excerpts of the design docs that name the conflict's files or symbols, the most relevant first.
 */
export async function baseDocs(root: string, base: string, anchors: { files: readonly string[]; symbols: readonly string[] }, maxChars: number): Promise<BaseDoc[]> {
  const listing = runGit(root, ["ls-tree", "-r", "-z", "--name-only", "--end-of-options", base], LIST_OPTIONS);
  if (listing.status !== 0) return [];
  const paths = listing.stdout.split("\0").filter(Boolean);
  const rules = paths.filter(path => DECISION_RULES.test(path));
  const topLevel = paths.filter(path => TOP_LEVEL_DOC.test(path)).sort((left, right) => left.length - right.length);
  const designDocs = paths.filter(path => isDesignDoc(path) && !TOP_LEVEL_DOC.test(path) && !isTestOrBenchPath(path)).slice(0, MAX_DOC_CANDIDATES);
  const blobs = await readGitBlobs(root, [...rules, ...topLevel, ...designDocs].map(path => `${base}:${path}`), { maxBlobBytes: MAX_SOURCE_BYTES, timeout: 60_000 });
  const text = (path: string) => blobs.get(`${base}:${path}`)?.toString("utf8");
  const docs: BaseDoc[] = [];
  let used = 0;
  const add = (path: string, body: string) => {
    if (body.trim() === "" || used + body.length > maxChars) return;
    docs.push({ path, text: body });
    used += body.length;
  };
  for (const path of rules) {
    try {
      const rule = JSON.parse(text(path) ?? "") as { id?: unknown; description?: unknown };
      if (typeof rule.description === "string") add(path, `Decision in force${typeof rule.id === "string" ? ` ${rule.id}` : ""}: ${rule.description}`);
    } catch {
      // A rule file that is not JSON states no decision.
    }
  }
  for (const path of topLevel) {
    const body = text(path);
    if (body !== undefined) add(path, body.length <= MAX_TOP_LEVEL_CHARS ? body : `${body.slice(0, MAX_TOP_LEVEL_CHARS)}\n[cut]`);
  }
  const terms = docTerms(anchors);
  if (terms.length === 0) return docs;
  const relevant = designDocs.flatMap(path => {
    const body = text(path);
    if (body === undefined) return [];
    const score = terms.reduce((sum, term) => sum + (body.includes(term) ? 1 : 0), 0);
    return score === 0 ? [] : [{ path, body, score }];
  }).sort((left, right) => right.score - left.score || left.path.localeCompare(right.path));
  for (const { path, body } of relevant) add(path, excerpts(body, terms));
  return docs;
}
