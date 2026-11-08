/**
 * The units of coordination one commit yields, read deterministically from its diff: hand-offs (open items and TODOs
 * that name a file or symbol), retirements (deleted and moved modules and code, removed exported declarations, and
 * "remove / retire / replace" statements), contract changes (an exported declaration's signature, a schema's tables, a
 * format file, an entry appended to a registry list), artefacts (a harness or tool other work should run rather than
 * rebuild), the design-doc and commit-message passages a judge reads positions from, the concept anchors a position
 * reaches through (`subsystemAnchors`, `conceptIds`), and the code fingerprints copy provenance compares across refs.
 * Each unit is anchored to the files and symbols it names; the ledger (`./ledger.ts`) qualifies it by branch and the
 * time it became visible.
 */
import { createHash } from "node:crypto";
import type { CommitChange, FileChange, LineBlock } from "./commits.js";
import { type Declaration, declarations, identifiers, shortName, sourceLanguageOf, sqlTables } from "./symbols.js";

export type UnitKind = "handoff" | "position" | "retirement" | "contract" | "copy" | "artefact";

export interface UnitAnchors {
  /** Repository paths; one ending in `/` covers everything under it. */
  readonly files: string[];
  readonly symbols: string[];
}

/** A unit as one commit yields it, before the ledger qualifies it by branch and time. */
export interface ExtractedUnit {
  readonly kind: UnitKind;
  /** One line a teammate reads: what the unit says, with where it was said. */
  readonly text: string;
  readonly anchors: UnitAnchors;
  /** A hand-off's file and normalised line: a later commit on the same ref that removes the line closes it. */
  readonly key: string | null;
}

/** Text a judge reads positions from: one design-doc passage a commit adds, its message, or a pull request's description. */
export interface Passage {
  /** `<sha>:<n>`, unique within a commit. */
  readonly id: string;
  /** The doc it is from; null for the commit message or a description. */
  readonly path: string | null;
  readonly heading: string | null;
  readonly text: string;
  /** Where a passage that is not a doc is from (`pull request #12`); absent for the commit message. */
  readonly source?: string;
}

/** A path in the ref's tree for a token a text names (`schema.rs`, `src/a/`), or undefined. */
export type PathResolver = (token: string) => string | undefined;

/** The longest unit text, in characters. */
export const MAX_UNIT_TEXT = 320;
/** The most symbols one unit is anchored to. */
const MAX_UNIT_SYMBOLS = 12;
const MAX_UNIT_FILES = 12;
/** The most passages one commit offers the position judge, and the longest. */
export const MAX_COMMIT_PASSAGES = 24;
export const MAX_PASSAGE_CHARS = 1_500;
const MIN_PASSAGE_CHARS = 60;
/** Consecutive normalised code lines one fingerprint covers. */
export const FINGERPRINT_WINDOW = 6;
const MAX_COMMIT_FINGERPRINTS = 4_000;

/**
 * A hand-off marker: TODO, FIXME, an open-item id or "open item" where a comment, heading or list item starts, or an
 * explicit open-item reference (`open item O18`) anywhere. A word like "follow-up" mid-sentence is not one.
 */
const HANDOFF_MARKER = /^\s*(?:\/\/+!?|#+|\/?\*+|--|<!--|[-*+]|\d+[.)]|\|)?\s*(?:\*\*)?(?:TODO|FIXME|XXX|O\d{1,3}\b|open (?:item|question)s?\b|decision needed|TBD\b)|\bopen items? O\d{1,3}\b/u;
const HANDOFF_HEADING = /\b(?:open items?|open questions?|follow[- ]?ups?|todos?|next steps?|hand[- ]?offs?|left to do|remaining work|known gaps?|outstanding|pending decisions?|decisions? needed)\b/iu;
/** A doc section that is itself an open item: `O61. The builtin catalogue`. */
const OPEN_ITEM_HEADING = /^O\d{1,3}\b/u;
/** Docs whose every passage is a design statement worth reading: decision records, plans, PRDs, open-item ledgers. */
const DESIGN_PATH = /(?:^|\/)(?:decisions?|adrs?|rfcs?)\/|open[-_]?items|(?:^|[/_-])(?:plan|design|prd|shape|invariants|architecture)[^/]*\.\w+$/iu;
const RETIREMENT_LINE = /\b(?:remov(?:e|es|ed|ing)|retir(?:e|es|ed|ing)|deprecat(?:e|es|ed|ing)|replac(?:e|es|ed|ing)|drop(?:s|ped|ping)?|delet(?:e|es|ed|ing)|supersed(?:e|es|ed|ing)|rip(?:s|ped)? out|no longer|obsolete|sunset)\b/iu;
const DECISION_LINE = /\b(?:decid\w*|decisions?|we (?:will|won't|do not|don't|never|always|keep|use|chose|choose)|must(?:n't| not)?|never|always|instead of|rather than|in favou?r of|chose|choose|prefer\w*|should(?:n't| not)?|shall|stance|invariant|source of truth|system of record|no longer|is the (?:only|single)|owns?|deprecat\w*|retir\w*|replac\w*|remov\w*|delet\w*|supersed\w*|obsolete|design)\b/iu;
const FORMAT_PATH = /(?:^|\/)(?:[^/]*schema[^/]*|migrations?\/[^/]+|[^/]*migrations?[^/]*\.\w+|[^/]*\.(?:sql|proto|graphql|gql|avsc|fbs|thrift)|[^/]*openapi[^/]*)$/iu;
const DOC_PATH = /\.(?:md|mdx|markdown|rst|adoc)$/iu;
const NOT_DESIGN_DOC = /(?:^|\/)(?:CHANGELOG|CHANGES|HISTORY|LICENSE|NOTICE|CODE_OF_CONDUCT|SECURITY|THIRD_PARTY)[^/]*$/iu;
const HEADING = /^\s{0,3}#{1,6}\s+(.*)$/u;
const LIST_OR_ROW = /^\s*(?:[-*+]|\d+[.)]|\|)\s*\S/u;
/** A list item or table row that says it is settled: a closed status cell, a ticked box, struck-through text. */
const SETTLED_ROW = /\|\s*(?:\*\*)?(?:closed|done|resolved|fixed|superseded|dropped|merged)\b|^\s*[-*+]\s*\[[xX]\]|^\s*(?:[-*+]|\d+[.)]|\|)?\s*~~/iu;
const TABLE_SEPARATOR = /^\s*\|?\s*:?-{3,}:?\s*(?:\|\s*:?-{3,}:?\s*)*\|?\s*$/u;
/** Tests and fixtures: code no other code depends on. */
const TEST_PATH = /(?:^|\/)(?:tests?|__tests__|specs?|fixtures?|testdata)\/|[._-](?:test|tests|spec)\.\w+$|_tests?\.rs$|(?:^|\/)test_[^/]*\.py$/iu;
/** Benchmarks and profiling scripts: measurement code, not an interface. */
const BENCH_PATH = /(?:^|\/)(?:bench(?:mark)?(?:es|s)?|profiling)\//iu;
/** Harnesses, benchmarks and tools: what other work runs rather than rebuilds. */
const HARNESS_PATH = /(?:^|\/)(?:bench(?:mark)?(?:es|s)?|harness(?:es)?|tools?|conformance)\/|harness/iu;

/** Whether `path` is prose a design position can live in: a doc, ADR or PRD, not a changelog or licence. */
export function isDesignDoc(path: string): boolean {
  return DOC_PATH.test(path) && !NOT_DESIGN_DOC.test(path);
}

/** Written the way code names things: snake case, camel case, or a constant with an underscore. */
function codeShaped(name: string): boolean {
  return /[a-z0-9]_[a-z0-9]/iu.test(name) || /[a-z][A-Z]/u.test(name) || /^[A-Z][A-Z0-9]*_[A-Z0-9_]+$/u.test(name);
}

/** Commit hashes, timestamps and numbered ids: tokens shaped like names that anchor nothing. */
function idLike(name: string): boolean {
  return /^[0-9a-f]{7,40}$/iu.test(name) || name.replace(/\D/gu, "").length * 2 >= name.length;
}

/** A declared name distinctive enough to anchor a unit: code-shaped, an all-caps constant, or a long name. */
export function distinctiveSymbol(name: string): boolean {
  const short = shortName(name);
  if (short.length < 4 || idLike(short)) return false;
  return codeShaped(short) || (/^[A-Z][A-Z0-9]+$/u.test(short) && short.length >= 5) || short.length >= 8;
}

function cutText(text: string, limit: number): string {
  const flat = text.replace(/\s+/gu, " ").trim();
  return flat.length <= limit ? flat : `${flat.slice(0, limit - 1)}…`;
}

function unique(values: Iterable<string>, limit: number): string[] {
  return [...new Set(values)].slice(0, limit);
}

const BACKTICKED = /`([^`\n]{2,160})`/gu;
const PATH_TOKEN = /(?<![\w/.@-])((?:[\w.-]+\/)+[\w.-]*|[\w-]+\.(?:ts|tsx|js|jsx|mjs|cjs|py|go|rs|md|sql|toml|json|ya?ml|proto|rr))(?::\d+(?:-\d+)?)?(?![\w/])/gu;
const QUALIFIED = /^[A-Za-z_$][\w$]*(?:(?:::|\.)[A-Za-z_$][\w$]*)*(?:\(\)|!)?$/u;
const CODE_IDENTIFIER = /(?<![\w$./-])([A-Za-z_][\w]*(?:::[A-Za-z_]\w*)*)(?=\s*\(|::|(?![\w$]))/gu;

/**
 * The files and symbols `text` names: backticked spans, path-like tokens (resolved against the ref's tree by
 * `resolve`) and code-shaped identifiers (`put_tape`, `rankPrompt`, `SCHEMA_VERSION`, `Store::open`).
 */
export function namedAnchors(text: string, resolve: PathResolver): UnitAnchors {
  const files: string[] = [];
  const symbols: string[] = [];
  const addPath = (token: string): boolean => {
    const cleaned = token.replace(/^(?:\.\/|[ab]\/)/u, "").replace(/:\d+(?:-\d+)?$/u, "").replace(/[.,;:]+$/u, "");
    if (cleaned.length < 3) return false;
    const path = resolve(cleaned);
    if (path !== undefined) files.push(path);
    return path !== undefined;
  };
  const addSymbol = (token: string, backticked: boolean): void => {
    const name = token.replace(/(?:\(\)|!)$/u, "").replace(/::/gu, ".");
    const short = shortName(name);
    if (short.length < 4 || idLike(short)) return;
    const qualified = name !== short || token.endsWith("()");
    if (codeShaped(short) || token.includes("::") || (backticked && (qualified || /^[A-Z][A-Za-z0-9]{4,}$/u.test(short)))) symbols.push(name);
  };
  for (const match of text.matchAll(BACKTICKED)) {
    const span = match[1]!.trim();
    if (QUALIFIED.test(span) && !/\.(?:ts|tsx|js|py|go|rs|md|sql|toml|json|ya?ml)$/u.test(span)) addSymbol(span, true);
    else if (!addPath(span)) {
      for (const name of identifiers(span)) addSymbol(name, false);
    }
  }
  const plain = text.replace(BACKTICKED, " ");
  for (const match of plain.matchAll(PATH_TOKEN)) addPath(match[1]!);
  for (const match of plain.replace(PATH_TOKEN, " ").matchAll(CODE_IDENTIFIER)) {
    const token = match[1]!;
    if (token.includes("::") || codeShaped(token)) addSymbol(token, false);
  }
  return { files: unique(files, MAX_UNIT_FILES), symbols: unique(symbols, MAX_UNIT_SYMBOLS) };
}

/** Words a subsystem's name carries that name no folder: "the SQL engine" lives in `sql/`. */
const GENERIC_SUBSYSTEM_WORDS = new Set(["the", "a", "an", "engine", "subsystem", "layer", "module", "component", "system", "pipeline", "crate", "package", "service", "library"]);
/** More anchors than this at a name's shallowest depth name no one home. */
const MAX_SUBSYSTEM_ANCHORS = 3;

/**
 * A tree's folders and files by their own name folded (`content_store/` and `content-store.ts` → `contentstore`): each
 * folder as a folder anchor (`a/b/`), each file by its stem (`facts_memo.rs` → `factsmemo`).
 */
export type NameIndex = ReadonlyMap<string, readonly string[]>;

export function nameIndex(paths: readonly string[]): NameIndex {
  const names = new Map<string, Set<string>>();
  const add = (name: string, anchor: string): void => {
    const key = name.toLowerCase().replace(/[^a-z0-9]/gu, "");
    if (key.length < 3) return;
    const set = names.get(key) ?? new Set<string>();
    set.add(anchor);
    names.set(key, set);
  };
  for (const path of paths) {
    const parts = path.split("/");
    for (let end = 1; end < parts.length; end += 1) add(parts[end - 1]!, `${parts.slice(0, end).join("/")}/`);
    add(parts.at(-1)!.replace(/\..*$/u, ""), path);
  }
  return new Map([...names].map(([key, set]) => [key, [...set].sort()]));
}

/**
 * The folders and files a subsystem's name names (`content store` → `native/src/content_store/`, `data-layer quality
 * policy` → `conformance/data-layer/`, `packs/data-layer/`): the first of its whole name, its name without generic
 * words ("engine", "layer"), then its runs of two or more words, longest first, whose own name it is, at the shallowest
 * depth. A run naming more than {@link MAX_SUBSYSTEM_ANCHORS} homes there is ambiguous and passed over. A single word
 * counts only when it is the whole name.
 */
export function subsystemAnchors(name: string, index: NameIndex): string[] {
  const words = name.toLowerCase().split(/[^a-z0-9]+/u).filter(Boolean);
  const forms = [words.join(""), words.filter(word => !GENERIC_SUBSYSTEM_WORDS.has(word)).join("")];
  for (let size = words.length - 1; size >= 2; size -= 1) {
    for (let start = 0; start + size <= words.length; start += 1) forms.push(words.slice(start, start + size).join(""));
  }
  const depth = (anchor: string): number => anchor.replace(/\/$/u, "").split("/").length;
  for (const form of new Set(forms)) {
    const found = form.length >= 3 ? index.get(form) : undefined;
    if (found === undefined) continue;
    const shallowest = Math.min(...found.map(depth));
    const home = found.filter(anchor => depth(anchor) === shallowest);
    if (home.length <= MAX_SUBSYSTEM_ANCHORS) return home;
  }
  return [];
}

/**
 * The decision-record numbers and open-item ids of `ids` that `text` writes, canonical: `O41`, and a decision's
 * number zero-padded to four digits (`decision 35` → `0035`). Anything else, or an id the text does not write, drops.
 */
export function conceptIds(ids: readonly string[], text: string): string[] {
  const kept: string[] = [];
  for (const raw of ids) {
    const openItem = /^O(\d{1,3})$/iu.exec(raw.trim());
    if (openItem !== null) {
      const id = `O${openItem[1]}`;
      if (new RegExp(`\\b${id}\\b`, "u").test(text)) kept.push(id);
      continue;
    }
    const decision = /^(?:decision\s*|ADR[-\s]?)?#?(\d{1,4})$/iu.exec(raw.trim());
    if (decision === null) continue;
    const number = Number(decision[1]);
    if (new RegExp(`(?<![\\w.])0*${number}(?![\\w.])`, "u").test(text)) kept.push(String(number).padStart(4, "0"));
  }
  return [...new Set(kept)];
}

function normaliseLine(line: string): string {
  return line.replace(/\s+/gu, " ").trim().toLowerCase();
}

/** The heading a line of `lines` (1-based `line`) sits under, or null. */
function headingAbove(lines: readonly string[], line: number): string | null {
  for (let index = Math.min(line - 1, lines.length - 1); index >= 0; index -= 1) {
    const heading = HEADING.exec(lines[index]!);
    if (heading !== null) return heading[1]!.trim();
  }
  return null;
}

/** The before and after texts of a commit's files that a unit extractor reads; missing ones read as absent. */
export interface CommitTexts {
  /** At the parent, by old path. */
  readonly before: ReadonlyMap<string, string>;
  /** At the commit, by path. */
  readonly after: ReadonlyMap<string, string>;
}

/** Hand-off units: added lines that leave work or a decision open and name what it concerns. */
export function handoffUnits(change: CommitChange, texts: CommitTexts, resolve: PathResolver): ExtractedUnit[] {
  const units: ExtractedUnit[] = [];
  const sections = new Map<string, { path: string; lines: string[] }>();
  for (const file of change.files) {
    const doc = DOC_PATH.test(file.path);
    const code = sourceLanguageOf(file.path) !== null;
    const afterLines = doc ? (texts.after.get(file.path) ?? "").split("\n") : [];
    for (const block of file.added) {
      block.lines.forEach((line, offset) => {
        if (line.trim().length < 6 || !/[A-Za-z]{3}/u.test(line) || SETTLED_ROW.test(line)) return;
        // A table's header row: the line after it in the file is the separator.
        if (doc && /^\s*\|/u.test(line) && TABLE_SEPARATOR.test(afterLines[block.start + offset] ?? "")) return;
        const heading = doc ? headingAbove(afterLines, block.start + offset) ?? "" : "";
        if (doc && OPEN_ITEM_HEADING.test(heading)) {
          const section = sections.get(heading) ?? { path: file.path, lines: [] };
          section.lines.push(line);
          sections.set(heading, section);
          return;
        }
        const underHeading = doc && LIST_OR_ROW.test(line) && HANDOFF_HEADING.test(heading);
        if (!underHeading && !HANDOFF_MARKER.test(line)) return;
        const named = namedAnchors(line, resolve);
        if (!code && named.files.length === 0 && named.symbols.length === 0 && !underHeading) return;
        units.push({
          kind: "handoff",
          text: cutText(`${line.trim()} (${file.path})`, MAX_UNIT_TEXT),
          anchors: { files: unique([file.path, ...named.files], MAX_UNIT_FILES), symbols: named.symbols },
          key: `${file.path}\u0000${normaliseLine(line)}`,
        });
      });
    }
  }
  // An open-item section a commit adds to or changes is one hand-off, keyed by its heading.
  for (const [heading, section] of sections) {
    const named = namedAnchors(section.lines.join("\n"), resolve);
    units.push({
      kind: "handoff",
      text: cutText(`${heading}: ${section.lines.find(line => line.trim().length > 0)?.trim() ?? ""} (${section.path})`, MAX_UNIT_TEXT),
      anchors: { files: unique([section.path, ...named.files], MAX_UNIT_FILES), symbols: named.symbols },
      key: `${section.path}\u0000#${heading.toLowerCase()}`,
    });
  }
  for (const line of change.body.split("\n")) {
    if (!HANDOFF_MARKER.test(line)) continue;
    const named = namedAnchors(line, resolve);
    if (named.files.length === 0 && named.symbols.length === 0) continue;
    units.push({ kind: "handoff", text: cutText(`${line.trim()} (commit message: ${change.subject})`, MAX_UNIT_TEXT), anchors: named, key: null });
  }
  return units;
}

/** The hand-off keys a commit closes: open-item and TODO lines it removes from a file. */
export function closedHandoffKeys(change: CommitChange): string[] {
  const keys: string[] = [];
  for (const file of change.files) {
    for (const block of file.removed) {
      for (const line of block.lines) {
        const heading = HEADING.exec(line);
        if (heading !== null && OPEN_ITEM_HEADING.test(heading[1]!.trim())) keys.push(`${file.oldPath}\u0000#${heading[1]!.trim().toLowerCase()}`);
        else if (line.trim().length >= 6 && (HANDOFF_MARKER.test(line) || LIST_OR_ROW.test(line))) keys.push(`${file.oldPath}\u0000${normaliseLine(line)}`);
      }
    }
  }
  return keys;
}

function exportedDeclarations(path: string, text: string | undefined): Declaration[] {
  const language = sourceLanguageOf(path);
  if (language === null || text === undefined) return [];
  return declarations(language, text).filter(declaration => declaration.exported);
}

/** The declaration names a commit's after texts make anywhere: a name removed here but declared there moved. */
function declaredAfter(change: CommitChange, texts: CommitTexts): Set<string> {
  const names = new Set<string>();
  for (const file of change.files) {
    const language = sourceLanguageOf(file.path);
    const text = texts.after.get(file.path);
    if (language !== null && text !== undefined) for (const declaration of declarations(language, text)) names.add(shortName(declaration.name));
  }
  return names;
}

/** The folder of `path`, as a folder anchor (`src/a/`); `` at the root. */
function folderOf(path: string): string {
  const slash = path.lastIndexOf("/");
  return slash === -1 ? "" : path.slice(0, slash + 1);
}

/** Retirement units: deleted and moved files, removed exported declarations, and statements that retire a named thing. */
export function retirementUnits(change: CommitChange, texts: CommitTexts, resolve: PathResolver): ExtractedUnit[] {
  const units: ExtractedUnit[] = [];
  const stillDeclared = declaredAfter(change, texts);
  // Deleted files, one unit per folder: a commit that drops a module tree is one retirement, not one per file.
  const deleted = new Map<string, Array<{ path: string; gone: string[] }>>();
  for (const file of change.files) {
    if (file.status !== "deleted") continue;
    const gone = exportedDeclarations(file.oldPath, texts.before.get(file.oldPath)).map(declaration => declaration.name).filter(name => !stillDeclared.has(shortName(name)));
    const folder = folderOf(file.oldPath);
    deleted.set(folder, [...(deleted.get(folder) ?? []), { path: file.oldPath, gone }]);
  }
  for (const [folder, files] of deleted) {
    const gone = files.flatMap(file => file.gone);
    const names = files.length === 1 ? files[0]!.path : `${files.length} files under ${folder || "the root"} (${files.slice(0, 4).map(file => file.path.slice(folder.length)).join(", ")}${files.length > 4 ? ", …" : ""})`;
    units.push({
      kind: "retirement",
      text: cutText(`deletes ${names}${gone.length > 0 ? `, with ${gone.slice(0, 6).join(", ")}` : ""}: ${change.subject}`, MAX_UNIT_TEXT),
      anchors: { files: files.length > MAX_UNIT_FILES && folder !== "" ? [folder] : files.map(file => file.path).slice(0, MAX_UNIT_FILES), symbols: unique(gone.filter(distinctiveSymbol), MAX_UNIT_SYMBOLS) },
      key: null,
    });
  }
  for (const file of change.files) {
    if (file.status === "deleted") continue;
    if (file.status === "renamed" && file.oldPath !== file.path) {
      units.push({ kind: "retirement", text: cutText(`moves ${file.oldPath} to ${file.path}: ${change.subject}`, MAX_UNIT_TEXT), anchors: { files: [file.oldPath, file.path], symbols: [] }, key: null });
    }
    const before = exportedDeclarations(file.oldPath, texts.before.get(file.oldPath));
    if (before.length === 0 || !texts.after.has(file.path)) continue;
    const removed = before.map(declaration => declaration.name).filter(name => !stillDeclared.has(shortName(name)));
    if (removed.length === 0) continue;
    units.push({
      kind: "retirement",
      text: cutText(`removes ${removed.slice(0, 6).join(", ")} from ${file.path}: ${change.subject}`, MAX_UNIT_TEXT),
      anchors: { files: [file.path], symbols: unique(removed, MAX_UNIT_SYMBOLS) },
      key: null,
    });
  }
  // Retirement statements come from the message only: a doc's statements reach the position judge as passages.
  const seen = new Set<string>();
  for (const line of [change.subject, ...change.body.split("\n")]) {
    if (!RETIREMENT_LINE.test(line) || seen.has(line)) continue;
    seen.add(line);
    const named = namedAnchors(line, resolve);
    if (named.files.length === 0 && named.symbols.length === 0) continue;
    units.push({ kind: "retirement", text: cutText(line === change.subject ? line : `${line.trim()} (${change.subject})`, MAX_UNIT_TEXT), anchors: named, key: null });
  }
  units.push(...movedCodeUnits(change, texts));
  return units;
}

/** Shared fingerprint windows between one file's removed lines and another's added lines that count as a move. */
const MIN_MOVED_WINDOWS = 2;

function windowHashes(blocks: readonly LineBlock[]): Set<string> {
  const hashes = new Set<string>();
  for (const block of blocks) {
    const lines = block.lines.map(fingerprintLine).filter((line): line is string => line !== null);
    for (let start = 0; start + FINGERPRINT_WINDOW <= lines.length; start += 1) hashes.add(createHash("sha1").update(lines.slice(start, start + FINGERPRINT_WINDOW).join("\n")).digest("hex").slice(0, 16));
  }
  return hashes;
}

/**
 * Code moved between files without a Git rename: a source file's removed lines reappearing among another file's added
 * lines. Anchored to both files and to the declarations that moved with it.
 */
export function movedCodeUnits(change: CommitChange, texts: CommitTexts): ExtractedUnit[] {
  const sources = change.files.filter(file => sourceLanguageOf(file.oldPath) !== null && file.removed.length > 0);
  const targets = change.files.filter(file => sourceLanguageOf(file.path) !== null && file.added.length > 0);
  if (sources.length === 0 || targets.length === 0) return [];
  const added = new Map(targets.map(file => [file.path, windowHashes(file.added)]));
  const units: ExtractedUnit[] = [];
  for (const source of sources) {
    const removed = windowHashes(source.removed);
    if (removed.size === 0) continue;
    for (const target of targets) {
      if (target.path === source.oldPath || (source.status === "renamed" && target.path === source.path)) continue;
      let shared = 0;
      for (const hash of added.get(target.path)!) if (removed.has(hash)) shared += 1;
      if (shared < MIN_MOVED_WINDOWS) continue;
      const language = sourceLanguageOf(target.path)!;
      const before = new Set(declarations(sourceLanguageOf(source.oldPath)!, texts.before.get(source.oldPath) ?? "").map(declaration => shortName(declaration.name)));
      const moved = declarations(language, texts.after.get(target.path) ?? "").map(declaration => declaration.name).filter(name => before.has(shortName(name)) && distinctiveSymbol(name));
      units.push({
        kind: "retirement",
        text: cutText(`moves code from ${source.oldPath} to ${target.path}${moved.length > 0 ? ` (${moved.slice(0, 6).join(", ")})` : ""}: ${change.subject}`, MAX_UNIT_TEXT),
        anchors: { files: [source.oldPath, target.path], symbols: unique(moved, MAX_UNIT_SYMBOLS) },
        key: null,
      });
    }
  }
  return units;
}

/** A signature as contracts compare it: visibility and spacing ignored, since neither changes what a caller writes. */
function signatureKey(signature: string): string {
  return signature
    .replace(/^(?:export\s+(?:default\s+)?|pub(?:\([^)]*\))?\s+|declare\s+)+/u, "")
    .replace(/\s+/gu, " ")
    .replace(/,\s*([)\]}>])/gu, "$1")
    .replace(/\s*([(),:<>{}[\]=|&])\s*/gu, "$1")
    .trim();
}

/** An entry's key in a registry list, written where its line starts: `id: "rule-approvals",`, `name = 'v3'`. */
const ENTRY_KEY = /^\s*(?:id|name|key|version)\s*[:=]\s*["'`]([\w.:/-]{2,80})["'`]/gmu;

/** The doc comment directly above 1-based `line` of `lines`, markers stripped, or null. */
function docCommentAbove(lines: readonly string[], line: number): string | null {
  const kept: string[] = [];
  for (let index = line - 2; index >= 0; index -= 1) {
    const comment = /^\s*(?:\/\/[!/]?|\/\*\*?|\*(?!\/)|#)\s?(.*?)\s*(?:\*\/)?$/u.exec(lines[index]!);
    if (comment === null) break;
    kept.unshift(comment[1]!);
  }
  const text = kept.join(" ").replace(/\s+/gu, " ").trim();
  return text === "" ? null : text;
}

/**
 * Entries a commit appends to an exported registry list in a schema or migrations file (a migration step, a schema
 * version), with the list's doc comment, which states the registry's rule ("Append only"): a list other branches add
 * to in the same order.
 */
function appendedEntryUnits(change: CommitChange, file: FileChange, after: string | undefined): ExtractedUnit[] {
  const language = sourceLanguageOf(file.path);
  if (language === null || after === undefined || !FORMAT_PATH.test(file.path)) return [];
  const all = declarations(language, after).filter(declaration => !declaration.name.includes("."));
  const lines = after.split("\n");
  const byList = new Map<Declaration, string[]>();
  for (const block of file.added) {
    const list = all.filter(declaration => declaration.line <= block.start).at(-1);
    if (list === undefined || !list.exported || list.kind !== "constant" || !/\[\s*$|\[\]|Array</u.test(list.signature)) continue;
    const keys = [...block.lines.join("\n").matchAll(ENTRY_KEY)].map(match => match[1]!);
    if (keys.length > 0) byList.set(list, [...(byList.get(list) ?? []), ...keys]);
  }
  return [...byList].map(([list, keys]) => {
    const doc = docCommentAbove(lines, list.line);
    return {
      kind: "contract",
      text: cutText(`appends ${unique(keys, 6).join(", ")} to ${list.name} in ${file.path}${doc === null ? "" : ` ("${doc}")`} (${change.subject})`, MAX_UNIT_TEXT),
      anchors: { files: [file.path], symbols: [list.name] },
      key: null,
    };
  });
}

/**
 * Contract units: an exported declaration whose signature changed, a schema's tables, a format file's change, an entry
 * appended to a registry list (`appendedEntryUnits`). Tests and benchmarks declare nothing other code depends on, and a
 * one-letter table is a placeholder (`CREATE TABLE x(…)` declaring a virtual table's columns, a scratch table), not a
 * name anything reads.
 */
export function contractUnits(change: CommitChange, texts: CommitTexts): ExtractedUnit[] {
  const units: ExtractedUnit[] = [];
  for (const file of change.files) {
    if (file.status === "deleted" || TEST_PATH.test(file.path) || BENCH_PATH.test(file.path)) continue;
    const before = new Map(exportedDeclarations(file.oldPath, texts.before.get(file.oldPath)).map(declaration => [declaration.name, declaration]));
    for (const after of exportedDeclarations(file.path, texts.after.get(file.path))) {
      const previous = before.get(after.name);
      if (previous === undefined || signatureKey(previous.signature) === signatureKey(after.signature)) continue;
      units.push({
        kind: "contract",
        text: cutText(`changes ${after.kind} ${after.name} in ${file.path}: \`${previous.signature}\` → \`${after.signature}\` (${change.subject})`, MAX_UNIT_TEXT),
        anchors: { files: [file.path], symbols: [after.name] },
        key: null,
      });
    }
    const changed = [...file.added, ...file.removed].flatMap(block => block.lines).join("\n");
    const tables = sqlTables(changed).filter(table => shortName(table).length > 1);
    if (tables.length > 0 || FORMAT_PATH.test(file.path)) {
      units.push({
        kind: "contract",
        text: cutText(`changes the ${tables.length > 0 ? `schema of ${tables.slice(0, 6).join(", ")}` : "format"} in ${file.path} (${change.subject})`, MAX_UNIT_TEXT),
        anchors: { files: [file.path], symbols: unique(tables, MAX_UNIT_SYMBOLS) },
        key: null,
      });
    }
    units.push(...appendedEntryUnits(change, file, texts.after.get(file.path)));
  }
  return units;
}

/** The passages a position judge reads: design-doc runs a commit adds that state a choice, and a substantive message. */
export function positionPassages(change: CommitChange, texts: CommitTexts): Passage[] {
  const passages: Passage[] = [];
  const message = `${change.subject}\n\n${change.body}`.trim();
  if (change.body.length >= MIN_PASSAGE_CHARS && DECISION_LINE.test(message)) {
    passages.push({ id: `${change.sha}:0`, path: null, heading: null, text: message.slice(0, MAX_PASSAGE_CHARS) });
  }
  for (const file of change.files) {
    if (!isDesignDoc(file.path)) continue;
    const afterLines = (texts.after.get(file.path) ?? "").split("\n");
    let run: { start: number; end: number; lines: string[] } | null = null;
    const runs: Array<{ start: number; lines: string[] }> = [];
    for (const block of file.added) {
      if (run !== null && block.start - run.end <= 3) {
        run.lines.push(...block.lines);
        run.end = block.start + block.lines.length;
      } else {
        if (run !== null) runs.push(run);
        run = { start: block.start, end: block.start + block.lines.length, lines: [...block.lines] };
      }
    }
    if (run !== null) runs.push(run);
    // Decision records, plans and open-item ledgers are read whole; elsewhere a run must state a choice.
    const designDoc = DESIGN_PATH.test(file.path);
    for (const entry of runs) {
      // A long run is read in paragraph-aligned chunks, so a decision deep in a new doc is not cut off.
      let chunk: string[] = [];
      let chunkStart = entry.start;
      const flush = (): void => {
        const text = chunk.join("\n").trim();
        chunk = [];
        if (text.length < MIN_PASSAGE_CHARS || (!designDoc && !DECISION_LINE.test(text))) return;
        const number = passages.filter(passage => passage.path !== null).length + 1;
        passages.push({ id: `${change.sha}:${number}`, path: file.path, heading: headingAbove(afterLines, chunkStart), text: text.slice(0, MAX_PASSAGE_CHARS) });
      };
      entry.lines.forEach((line, offset) => {
        const size = chunk.reduce((sum, kept) => sum + kept.length + 1, 0);
        if (chunk.length > 0 && size + line.length > MAX_PASSAGE_CHARS && (line.trim() === "" || HEADING.test(line) || size > MAX_PASSAGE_CHARS * 0.8)) {
          flush();
          chunkStart = entry.start + offset;
        }
        if (chunk.length === 0) chunkStart = entry.start + offset;
        chunk.push(line);
      });
      flush();
    }
  }
  // Over the cap, chunks that state a choice go before the rest of a design doc.
  const fromMessage = passages.filter(passage => passage.path === null);
  const docs = passages.filter(passage => passage.path !== null);
  const ordered = [...docs.filter(passage => DECISION_LINE.test(passage.text)), ...docs.filter(passage => !DECISION_LINE.test(passage.text))];
  return [...fromMessage, ...ordered].slice(0, MAX_COMMIT_PASSAGES);
}

/** A code line as fingerprints compare it: trimmed, whitespace collapsed; blank, brace-only and comment lines drop. */
function fingerprintLine(line: string): string | null {
  const flat = line.replace(/\s+/gu, " ").trim();
  if (flat.length < 8 || /^[\s{}()[\];,]*$/u.test(flat) || /^(?:\/\/|#(?!\[)|\/\*|\*|--)/u.test(flat)) return null;
  return flat;
}

export interface CodeFingerprint {
  readonly path: string;
  /** Hex sha1 prefix of {@link FINGERPRINT_WINDOW} consecutive normalised added lines. */
  readonly hash: string;
}

/** Fingerprints of the code a commit adds: each window of consecutive normalised lines in a source file's added runs. */
export function codeFingerprints(change: CommitChange): CodeFingerprint[] {
  const found: CodeFingerprint[] = [];
  const seen = new Set<string>();
  for (const file of change.files) {
    if (sourceLanguageOf(file.path) === null) continue;
    for (const block of file.added) {
      const lines = block.lines.map(fingerprintLine).filter((line): line is string => line !== null);
      for (let start = 0; start + FINGERPRINT_WINDOW <= lines.length; start += 1) {
        const hash = createHash("sha1").update(lines.slice(start, start + FINGERPRINT_WINDOW).join("\n")).digest("hex").slice(0, 16);
        const key = `${file.path}\u0000${hash}`;
        if (seen.has(key)) continue;
        seen.add(key);
        found.push({ path: file.path, hash });
        if (found.length >= MAX_COMMIT_FINGERPRINTS) return found;
      }
    }
  }
  return found;
}

/** The identifiers a commit's changed lines use, in every file: what the commit touches by name. */
export function touchedSymbols(files: readonly FileChange[]): Set<string> {
  const names = new Set<string>();
  for (const file of files) {
    for (const block of [...file.added, ...file.removed]) for (const name of identifiers(block.lines.join("\n"))) names.add(name);
  }
  return names;
}

/** The first sentence of a file's leading doc comment (`//!`, a block comment, `#`, a docstring, a doc's title), or null. */
export function leadingSummary(text: string): string | null {
  const lines: string[] = [];
  for (const raw of text.split("\n").slice(0, 40)) {
    const line = raw.trim();
    if (lines.length === 0 && (line === "" || line.startsWith("#!") || /^(?:"use |'use |\/\/ SPDX|# SPDX|<!--)/u.test(line))) continue;
    const comment = /^(?:\/\/[!/]?|\/\*\*?|\*(?!\/)|#{1,6}|"""|''')\s?(.*)$/u.exec(line);
    if (comment === null || line === "*/") break;
    const body = comment[1]!.replace(/\*\/$/u, "").replace(/"""|'''/gu, "").trim();
    if (body !== "") lines.push(body);
    if (/[.!?](?:\s|$)/u.test(body) && lines.join(" ").length >= 30) break;
  }
  const joined = lines.join(" ").replace(/\s+/gu, " ").trim();
  if (joined.length < 12) return null;
  const sentence = /^(.+?[.!?])(?:\s|$)/u.exec(joined);
  return (sentence?.[1] ?? joined).slice(0, 240);
}

/**
 * Artefact units: a harness, benchmark or tool a commit adds (a new source file on such a path, not a test), which
 * other work should run rather than rebuild. Summarised by the file's leading doc comment or the commit subject,
 * anchored to the file and its exported declarations. Any other new module or export is left out: on its own it says
 * nothing another branch depends on, and the docs and messages that frame one as reusable reach the position judge.
 */
export function artefactUnits(change: CommitChange, texts: CommitTexts): ExtractedUnit[] {
  const units: ExtractedUnit[] = [];
  for (const file of change.files) {
    if (file.status !== "added" || !HARNESS_PATH.test(file.path) || TEST_PATH.test(file.path)) continue;
    const language = sourceLanguageOf(file.path);
    const after = texts.after.get(file.path);
    if (language === null || after === undefined) continue;
    const names = declarations(language, after).filter(declaration => declaration.exported && declaration.kind !== "module").map(declaration => declaration.name);
    units.push({
      kind: "artefact",
      text: cutText(`adds ${file.path}: ${leadingSummary(after) ?? change.subject}${names.length > 0 ? ` (${names.slice(0, 6).join(", ")})` : ""}`, MAX_UNIT_TEXT),
      anchors: { files: [file.path], symbols: unique(names.filter(distinctiveSymbol), MAX_UNIT_SYMBOLS) },
      key: null,
    });
  }
  return units;
}

/** Every deterministic unit a commit yields: hand-offs, retirements, contract changes and artefacts. */
export function extractUnits(change: CommitChange, texts: CommitTexts, resolve: PathResolver): ExtractedUnit[] {
  if (change.merge) return [];
  return [...handoffUnits(change, texts, resolve), ...retirementUnits(change, texts, resolve), ...contractUnits(change, texts), ...artefactUnits(change, texts)];
}
