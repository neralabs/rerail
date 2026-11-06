/**
 * The units of coordination one commit yields, read deterministically from its diff: hand-offs (open items and TODOs
 * that name a file or symbol), retirements (deleted and moved modules, removed exported declarations, and "remove /
 * retire / replace" statements), contract changes (an exported declaration's signature, a schema's tables, a format
 * file), the design-doc and commit-message passages a judge reads positions from, and the code fingerprints copy
 * provenance compares across refs. Each unit is anchored to the files and symbols it names; the ledger
 * (`./ledger.ts`) qualifies it by branch and the time it became visible.
 */
import { createHash } from "node:crypto";
import type { CommitChange, FileChange } from "./commits.js";
import { type Declaration, declarations, identifiers, shortName, sourceLanguageOf, sqlTables } from "./symbols.js";

export type UnitKind = "handoff" | "position" | "retirement" | "contract" | "copy";

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

/** Text a judge reads positions from: one design-doc passage a commit adds, or its message. */
export interface Passage {
  /** `<sha>:<n>`, unique within a commit. */
  readonly id: string;
  /** The doc it is from; null for the commit message. */
  readonly path: string | null;
  readonly heading: string | null;
  readonly text: string;
}

/** A path in the ref's tree for a token a text names (`schema.rs`, `src/a/`), or undefined. */
export type PathResolver = (token: string) => string | undefined;

/** The longest unit text, in characters. */
export const MAX_UNIT_TEXT = 320;
/** The most symbols one unit is anchored to. */
const MAX_UNIT_SYMBOLS = 12;
const MAX_UNIT_FILES = 12;
/** The most passages one commit offers the position judge, and the longest. */
export const MAX_COMMIT_PASSAGES = 8;
export const MAX_PASSAGE_CHARS = 1_500;
const MIN_PASSAGE_CHARS = 60;
/** Consecutive normalised code lines one fingerprint covers. */
export const FINGERPRINT_WINDOW = 6;
const MAX_COMMIT_FINGERPRINTS = 4_000;

const HANDOFF_LINE = /\b(?:TODO|FIXME|XXX)\b|\bopen (?:item|question)s?\b|\bdecision needed\b|\bneeds? (?:a |an )?decision\b|\bleft (?:to|for) (?:the )?[\w-]+|\bfollow[- ]?ups?\b|\bTBD\b|\bto be decided\b|\bunresolved\b|\bblocked (?:on|by)\b|\bwaiting (?:on|for)\b|\bnot yet (?:done|implemented|wired|ported|handled|decided)\b|\bparked\b|\bhand[- ]?off\b|\bowner:/iu;
const HANDOFF_HEADING = /\b(?:open items?|open questions?|follow[- ]?ups?|todos?|next steps?|hand[- ]?offs?|left to do|remaining work|known gaps?|outstanding|pending decisions?|decisions? needed)\b/iu;
const RETIREMENT_LINE = /\b(?:remov(?:e|es|ed|ing)|retir(?:e|es|ed|ing)|deprecat(?:e|es|ed|ing)|replac(?:e|es|ed|ing)|drop(?:s|ped|ping)?|delet(?:e|es|ed|ing)|supersed(?:e|es|ed|ing)|rip(?:s|ped)? out|no longer|obsolete|sunset)\b/iu;
const DECISION_LINE = /\b(?:decid\w*|decisions?|we (?:will|won't|do not|don't|never|always|keep|use|chose|choose)|must(?:n't| not)?|never|always|instead of|rather than|in favou?r of|chose|choose|prefer\w*|should(?:n't| not)?|shall|stance|invariant|source of truth|system of record|no longer|is the (?:only|single)|owns?|deprecat\w*|retir\w*|replac\w*|design)\b/iu;
const FORMAT_PATH = /(?:^|\/)(?:[^/]*schema[^/]*|migrations?\/[^/]+|[^/]*\.(?:sql|proto|graphql|gql|avsc|fbs|thrift)|[^/]*openapi[^/]*)$/iu;
const DOC_PATH = /\.(?:md|mdx|markdown|rst|adoc)$/iu;
const NOT_DESIGN_DOC = /(?:^|\/)(?:CHANGELOG|CHANGES|HISTORY|LICENSE|NOTICE|CODE_OF_CONDUCT|SECURITY|THIRD_PARTY)[^/]*$/iu;
const HEADING = /^\s{0,3}#{1,6}\s+(.*)$/u;
const LIST_OR_ROW = /^\s*(?:[-*+]|\d+[.)]|\|)\s*\S/u;

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
  for (const file of change.files) {
    const doc = DOC_PATH.test(file.path);
    const code = sourceLanguageOf(file.path) !== null;
    const afterLines = doc ? (texts.after.get(file.path) ?? "").split("\n") : [];
    for (const block of file.added) {
      block.lines.forEach((line, offset) => {
        if (line.trim().length < 6) return;
        const underHeading = doc && LIST_OR_ROW.test(line) && HANDOFF_HEADING.test(headingAbove(afterLines, block.start + offset) ?? "");
        if (!underHeading && !HANDOFF_LINE.test(line)) return;
        const named = namedAnchors(line, resolve);
        if (!code && named.files.length === 0 && named.symbols.length === 0) return;
        units.push({
          kind: "handoff",
          text: cutText(`${line.trim()} (${file.path})`, MAX_UNIT_TEXT),
          anchors: { files: unique([file.path, ...named.files], MAX_UNIT_FILES), symbols: named.symbols },
          key: `${file.path}\u0000${normaliseLine(line)}`,
        });
      });
    }
  }
  for (const line of change.body.split("\n")) {
    if (!HANDOFF_LINE.test(line)) continue;
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
      for (const line of block.lines) if (line.trim().length >= 6 && (HANDOFF_LINE.test(line) || LIST_OR_ROW.test(line))) keys.push(`${file.oldPath}\u0000${normaliseLine(line)}`);
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
  // Table rows in docs are measurements and logs far more often than statements; they are left out.
  const docLines = change.files.filter(file => isDesignDoc(file.path)).flatMap(file => file.added.flatMap(block => block.lines)).filter(line => !/^\s*\|/u.test(line));
  const statements = [change.subject, ...change.body.split("\n"), ...docLines];
  const seen = new Set<string>();
  for (const line of statements) {
    if (!RETIREMENT_LINE.test(line) || seen.has(line)) continue;
    seen.add(line);
    const named = namedAnchors(line, resolve);
    if (named.files.length === 0 && named.symbols.length === 0) continue;
    units.push({ kind: "retirement", text: cutText(line === change.subject ? line : `${line.trim()} (${change.subject})`, MAX_UNIT_TEXT), anchors: named, key: null });
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

/** Contract units: an exported declaration whose signature changed, a schema's tables, a format file's change. */
export function contractUnits(change: CommitChange, texts: CommitTexts): ExtractedUnit[] {
  const units: ExtractedUnit[] = [];
  for (const file of change.files) {
    if (file.status === "deleted") continue;
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
    const tables = sqlTables(changed);
    if (tables.length > 0 || FORMAT_PATH.test(file.path)) {
      units.push({
        kind: "contract",
        text: cutText(`changes the ${tables.length > 0 ? `schema of ${tables.slice(0, 6).join(", ")}` : "format"} in ${file.path} (${change.subject})`, MAX_UNIT_TEXT),
        anchors: { files: [file.path], symbols: unique(tables, MAX_UNIT_SYMBOLS) },
        key: null,
      });
    }
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
    for (const entry of runs) {
      const text = entry.lines.join("\n").trim();
      if (text.length < MIN_PASSAGE_CHARS || !DECISION_LINE.test(text)) continue;
      const number = passages.filter(passage => passage.path !== null).length + 1;
      passages.push({ id: `${change.sha}:${number}`, path: file.path, heading: headingAbove(afterLines, entry.start), text: text.slice(0, MAX_PASSAGE_CHARS) });
    }
  }
  return passages.slice(0, MAX_COMMIT_PASSAGES);
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

/** Every deterministic unit a commit yields: hand-offs, retirements and contract changes. */
export function extractUnits(change: CommitChange, texts: CommitTexts, resolve: PathResolver): ExtractedUnit[] {
  if (change.merge) return [];
  return [...handoffUnits(change, texts, resolve), ...retirementUnits(change, texts, resolve), ...contractUnits(change, texts)];
}
