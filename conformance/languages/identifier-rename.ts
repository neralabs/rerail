/**
 * Metamorphic identifier rename for the identifier-poisoning gate (test/conformance/identifier-rename.test.ts): a port
 * that passes because it matches names only the upstream tool's tests use (`foo`, `MyClass`, ...) instead of the
 * structure the rule is about changes score when every user-chosen identifier in its fixtures is renamed to an opaque
 * one. ECMA family only: the rename reads the source with TypeScript's parser.
 *
 * - Same length: each renamed identifier keeps its UTF-16 length and character shape (upper stays upper, lower stays
 *   lower, digits stay digits, `_`/`$`/non-ASCII stay put, a Hungarian `I` before an uppercase letter stays), so every
 *   recorded finding still points at the same token and naming-convention rules judge the same shape.
 * - Consistent per file: one opaque name per distinct identifier text, applied to every occurrence (declarations,
 *   references, property names, shorthand, types, labels, user JSX tags). The map is a bijection, so every binding,
 *   shadowing and name-equality relation in the file is preserved without modelling scopes.
 * - Protected: keywords and contextual keywords used as identifiers, the ES standard library (globals and every
 *   member declared in TypeScript's lib.es*.d.ts), a curated host list (DOM/Node/React/module names), and what the
 *   tool's recording lists in `protected-identifiers.json` (for TSLint: identifier-like literals in each rule's
 *   implementation and the words and patterns of each test's options). Intrinsic JSX tags and attributes on intrinsic
 *   elements are protected; so is any name with an occurrence that cannot be rewritten in place (unicode escapes).
 *   String/number literal contents and comments are never touched.
 */
import { cpSync, existsSync, readdirSync, readFileSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join } from "node:path";
import ts from "typescript";
import { z } from "zod";
import { fixturesDirectory, fixturesRecordingPath, loadRecording, type LoadedPack } from "./pack.js";
import { compareDeterministicStrings } from "../../src/stable-json.js";

/** Globals and members a rule may legitimately target that are not in the ES lib: host, module system, React. */
const HOST_NAMES = [
  "window", "document", "navigator", "location", "globalThis", "self", "console", "process", "Buffer", "global", "require", "module", "exports",
  "define", "__dirname", "__filename", "setTimeout", "clearTimeout", "setInterval", "clearInterval", "setImmediate", "clearImmediate",
  "requestAnimationFrame", "fetch", "alert", "confirm", "prompt", "localStorage", "sessionStorage", "addEventListener", "removeEventListener",
  "log", "warn", "error", "info", "debug", "trace", "time", "timeEnd", "dir", "table", "assert",
  "React", "ReactDOM", "Component", "PureComponent", "Fragment", "createElement", "props", "state", "setState", "render", "children", "key",
  "ref", "className", "useState", "useEffect", "useMemo", "useCallback", "useRef", "useContext",
  "jQuery", "$", "_", "angular", "describe", "it", "expect", "test", "beforeEach", "afterEach",
  "arguments", "eval", "undefined", "NaN", "Infinity", "constructor", "prototype", "__proto__", "this", "super",
];

/** Identifiers that TypeScript parses as `Identifier` but that carry grammar meaning in some position. */
const CONTEXTUAL_KEYWORDS = [
  "abstract", "accessor", "any", "as", "asserts", "assert", "async", "await", "bigint", "boolean", "constructor", "declare", "from", "get", "global",
  "implements", "infer", "interface", "intrinsic", "is", "keyof", "let", "module", "namespace", "never", "number", "object", "of", "out", "override",
  "package", "private", "protected", "public", "readonly", "require", "satisfies", "set", "static", "string", "symbol", "type", "undefined",
  "unique", "unknown", "using", "yield",
];

let standardLibrary: ReadonlySet<string> | undefined;

/** Top-level declarations and every member name in TypeScript's lib.es5/es20xx/esnext declaration files. */
export function standardLibraryNames(): ReadonlySet<string> {
  if (standardLibrary !== undefined) return standardLibrary;
  const libDirectory = dirname(createRequire(import.meta.url).resolve("typescript/lib/lib.es5.d.ts"));
  const names = new Set<string>();
  const add = (name: ts.PropertyName | ts.BindingName | undefined): void => {
    if (name !== undefined && (ts.isIdentifier(name) || ts.isPrivateIdentifier(name))) names.add(name.text);
  };
  const visit = (node: ts.Node): void => {
    if (ts.isVariableDeclaration(node) || ts.isFunctionDeclaration(node) || ts.isInterfaceDeclaration(node) || ts.isTypeAliasDeclaration(node)) add(node.name);
    else if (ts.isModuleDeclaration(node) && ts.isIdentifier(node.name)) add(node.name);
    else if (ts.isPropertySignature(node) || ts.isMethodSignature(node) || ts.isPropertyDeclaration(node) || ts.isMethodDeclaration(node)) add(node.name);
    ts.forEachChild(node, visit);
  };
  for (const file of readdirSync(libDirectory).filter(name => /^lib\.(es5|es20\d\d(\.\w+)*|esnext(\.\w+)*)\.d\.ts$/.test(name))) {
    visit(ts.createSourceFile(file, readFileSync(join(libDirectory, file), "utf8"), ts.ScriptTarget.ESNext, false));
  }
  standardLibrary = names;
  return names;
}

/** Keywords (reserved and contextual), the standard library and the host list: protected for every rule. */
let globalNames: ReadonlySet<string> | undefined;
export function globalProtectedNames(): ReadonlySet<string> {
  if (globalNames !== undefined) return globalNames;
  const names = new Set<string>([...standardLibraryNames(), ...HOST_NAMES, ...CONTEXTUAL_KEYWORDS]);
  for (let kind = ts.SyntaxKind.FirstKeyword; kind <= ts.SyntaxKind.LastKeyword; kind += 1) {
    const text = ts.tokenToString(kind);
    if (text !== undefined) names.add(text);
  }
  globalNames = names;
  return names;
}

/** Beside a tool's fixtures recording: the names its rename keeps, by fixture path or directory. A pack opts in with it. */
export const PROTECTED_IDENTIFIERS_FILE = "protected-identifiers.json";

const protectedIdentifiersSchema = z.record(z.string(), z.strictObject({ names: z.array(z.string()), patterns: z.array(z.string()) }));
type ProtectedIdentifiers = z.infer<typeof protectedIdentifiersSchema>;

/** The tools of `loaded` whose fixtures the gate renames: those recording protected identifiers. */
export function renamedTools(loaded: LoadedPack): string[] {
  return [...new Set(loaded.pack.rules.map(rule => rule.upstream.tool))]
    .filter(tool => existsSync(join(loaded.oraclesDirectory, tool, PROTECTED_IDENTIFIERS_FILE)))
    .sort(compareDeterministicStrings);
}

/** One fixture's protection: the global names plus every recorded entry for the file or a directory above it. */
function recordedProtection(recorded: ProtectedIdentifiers, path: string): Protection {
  const entries = Object.entries(recorded).filter(([key]) => path === key || path.startsWith(`${key}/`));
  return {
    names: new Set([...globalProtectedNames(), ...entries.flatMap(([, entry]) => entry.names)]),
    patterns: entries.flatMap(([, entry]) => entry.patterns.map(pattern => new RegExp(pattern))),
  };
}

export interface RenamedFixture {
  path: string;
  source: string;
  renamed: RenameResult;
  /** Why the rename may change more than names: a changed length or TypeScript parse, names left unrenamed. */
  problems: string[];
}

/** Every fixture of `tool`'s recording, renamed under its recorded protection. */
export function renameFixtures(loaded: LoadedPack, tool: string): RenamedFixture[] {
  const recorded = protectedIdentifiersSchema.parse(JSON.parse(readFileSync(join(loaded.oraclesDirectory, tool, PROTECTED_IDENTIFIERS_FILE), "utf8")));
  const recording = loadRecording(fixturesRecordingPath(loaded, tool));
  if (recording.source.kind !== "fixtures") throw new Error(`${tool}: not a fixtures recording`);
  return recording.source.files.flatMap(unit => (typeof unit === "string" ? [unit] : unit)).map(path => {
    const source = readFileSync(join(fixturesDirectory(loaded, tool), path), "utf8");
    const renamed = renameIdentifiers(source, path, recordedProtection(recorded, path));
    const problems: string[] = [];
    if (renamed.source.length !== source.length) problems.push("length changed");
    const before = syntaxDiagnostics(source, path).join(" ");
    const after = syntaxDiagnostics(renamed.source, path).join(" ");
    if (before !== after) problems.push(`TypeScript syntax diagnostics changed: [${before}] -> [${after}]`);
    if (renamed.exhausted.length > 0) problems.push(`no free same-shape name for ${renamed.exhausted.join(", ")}`);
    return { path, source, renamed, problems };
  });
}

/**
 * `loaded` with its recordings copied to `directory` and each tool's fixtures in `renamed` replaced by their renamed
 * text: the same findings, which still point at the same tokens, over sources whose user identifiers are opaque.
 */
export function renamedPack(loaded: LoadedPack, renamed: ReadonlyMap<string, readonly RenamedFixture[]>, directory: string): LoadedPack {
  cpSync(loaded.oraclesDirectory, directory, { recursive: true });
  const copy = { ...loaded, oraclesDirectory: directory };
  for (const [tool, fixtures] of renamed) for (const fixture of fixtures) writeFileSync(join(fixturesDirectory(copy, tool), fixture.path), fixture.renamed.source);
  return copy;
}

/** What the renamer must leave alone: exact names, and patterns an identifier may match. */
export interface Protection {
  names: ReadonlySet<string>;
  patterns: readonly RegExp[];
}

export function isProtected(protection: Protection, name: string): boolean {
  return protection.names.has(name) || protection.patterns.some(pattern => pattern.test(name));
}

export interface RenameResult {
  source: string;
  /** Original identifier text to its opaque replacement. */
  renames: Map<string, string>;
  /** Names that would be renamed but have no free same-shape replacement (e.g. every one-letter name is taken). */
  exhausted: string[];
}

interface Occurrence {
  start: number;
  /** Offset of the renamed part within the token (`#` of a private name is kept). */
  offset: number;
}

function fnv1a(text: string): number {
  let hash = 0x811c9dc5;
  for (let index = 0; index < text.length; index += 1) {
    hash ^= text.charCodeAt(index);
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash;
}

function xorshift(seed: number): () => number {
  let state = seed || 1;
  return () => {
    state ^= state << 13;
    state >>>= 0;
    state ^= state >>> 17;
    state ^= state << 5;
    state >>>= 0;
    return state;
  };
}

const UPPER = "ABCDEFGHIJKLMNOPQRSTUVWXYZ";
const LOWER = "abcdefghijklmnopqrstuvwxyz";
const DIGITS = "0123456789";

/** A deterministic same-shape candidate for `name` (attempt `attempt`). */
function candidate(name: string, attempt: number): string {
  const next = xorshift(fnv1a(`${name}\u0000${attempt}`));
  const keepHungarian = /^I[A-Z]/.test(name);
  let result = "";
  for (let index = 0; index < name.length; index += 1) {
    const char = name[index]!;
    if (index === 0 && keepHungarian) result += char;
    else if (UPPER.includes(char)) result += UPPER[next() % 26];
    else if (LOWER.includes(char)) result += LOWER[next() % 26];
    else if (DIGITS.includes(char)) result += DIGITS[next() % 10];
    else result += char;
  }
  return result;
}

function isIntrinsicTag(name: ts.JsxTagNameExpression): boolean {
  return ts.isIdentifier(name) && /^[a-z]/.test(name.text) && !name.text.includes(".");
}

/**
 * A string literal naming a property or member: a declared name (`"a"() {}`, `{ "a": 1 }`), a computed name (`["a"]`)
 * or an element access (`o["a"]`). TSLint compares these with identifier names, so they follow the identifier's rename.
 */
function isNameLiteral(node: ts.StringLiteral): boolean {
  const parent = node.parent;
  if (ts.isComputedPropertyName(parent)) return true;
  if (ts.isElementAccessExpression(parent)) return parent.argumentExpression === node;
  const declaration = parent as ts.Node & { name?: ts.Node };
  return declaration.name === node && !ts.isModuleDeclaration(parent);
}

function scriptKind(fileName: string): ts.ScriptKind {
  return /\.[jt]sx$/.test(fileName) ? ts.ScriptKind.TSX : ts.ScriptKind.TS;
}

/** Rename every user-chosen identifier in `source` (see module doc). `fileName` picks TS or TSX parsing. */
export function renameIdentifiers(source: string, fileName: string, protection: Protection): RenameResult {
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.ESNext, true, scriptKind(fileName));
  const occurrences = new Map<string, Occurrence[]>();
  /** String literals in name position (`"a"() {}`, `["a"]: 1`, `o["a"]`): content start offsets by text. */
  const nameLiterals = new Map<string, number[]>();
  const pinned = new Set<string>();
  const allTexts = new Set<string>();
  const visit = (node: ts.Node): void => {
    if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node)) {
      const text = node.text;
      if (text === "") return;
      allTexts.add(text);
      const start = node.getStart(file);
      const parent = node.parent;
      const intrinsicTag = (ts.isJsxOpeningElement(parent) || ts.isJsxSelfClosingElement(parent) || ts.isJsxClosingElement(parent)) && parent.tagName === node && isIntrinsicTag(parent.tagName);
      const intrinsicAttribute = ts.isJsxAttribute(parent) && parent.name === node && isIntrinsicTag(parent.parent.parent.tagName);
      const keywordLike = ts.isIdentifier(node) && ts.identifierToKeywordKind(node) !== undefined;
      if (source.slice(start, node.end) !== text || intrinsicTag || intrinsicAttribute || keywordLike) pinned.add(text);
      const bucket = occurrences.get(text) ?? [];
      bucket.push({ start, offset: ts.isPrivateIdentifier(node) ? 1 : 0 });
      occurrences.set(text, bucket);
    }
    else if (ts.isStringLiteral(node) && isNameLiteral(node)) {
      const bucket = nameLiterals.get(node.text) ?? [];
      bucket.push(node.getStart(file) + 1);
      nameLiterals.set(node.text, bucket);
      if (source.slice(node.getStart(file) + 1, node.end - 1) !== node.text) pinned.add(node.text);
    }
    ts.forEachChild(node, visit);
  };
  visit(file);

  const bareName = (name: string): string => (name.startsWith("#") ? name.slice(1) : name);
  const renamable = [...occurrences.keys()]
    .filter(name => !pinned.has(name) && !isProtected(protection, bareName(name)) && /[A-Za-z]/.test(bareName(name).slice(/^I[A-Z]/.test(bareName(name)) ? 1 : 0)))
    .sort(compareDeterministicStrings);
  const renamableSet = new Set(renamable);
  // Targets may reuse another renamed name (a permutation keeps the map a bijection), never a name that stays.
  const staying = new Set([...allTexts].filter(text => !renamableSet.has(text)).map(bareName));
  // Words in comments and strings are avoided too, except for one- and two-letter names, whose shapes run out.
  const words = new Set((source.match(/[A-Za-z_$][\w$]*/g) ?? []).filter(word => !allTexts.has(word)));
  const assigned = new Set<string>();
  const renames = new Map<string, string>();
  const exhausted: string[] = [];
  for (const name of renamable) {
    const bare = bareName(name);
    let replacement: string | undefined;
    for (let attempt = 0; attempt < 500 && replacement === undefined; attempt += 1) {
      const next = candidate(bare, attempt);
      const blocked = staying.has(next) || isProtected(protection, next) || assigned.has(next) || (bare.length > 2 && words.has(next));
      if (next !== bare && !blocked) replacement = next;
    }
    if (replacement === undefined) {
      exhausted.push(name);
      continue;
    }
    assigned.add(replacement);
    renames.set(name, name.startsWith("#") ? `#${replacement}` : replacement);
  }

  const edits: Array<{ start: number; text: string }> = [];
  for (const [name, replacement] of renames) {
    for (const { start, offset } of occurrences.get(name)!) edits.push({ start: start + offset, text: replacement.slice(offset) });
    for (const start of nameLiterals.get(name) ?? []) edits.push({ start, text: replacement });
  }
  edits.sort((left, right) => left.start - right.start);
  let result = "";
  let cursor = 0;
  for (const edit of edits) {
    result += source.slice(cursor, edit.start) + edit.text;
    cursor = edit.start + edit.text.length;
  }
  result += source.slice(cursor);
  return { source: result, renames, exhausted };
}

/** TypeScript's syntax diagnostics for `source` (count and codes), to check a rename did not change how it parses. */
export function syntaxDiagnostics(source: string, fileName: string): string[] {
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.ESNext, true, scriptKind(fileName));
  const program = ts.createProgram({
    rootNames: [fileName],
    options: { noEmit: true, noLib: true, types: [], jsx: ts.JsxEmit.Preserve },
    host: {
      ...ts.createCompilerHost({}),
      getSourceFile: name => (name === fileName ? file : undefined),
      fileExists: name => name === fileName,
      readFile: name => (name === fileName ? source : undefined),
    },
  });
  return program.getSyntacticDiagnostics(file).map(diagnostic => `${diagnostic.start ?? -1}:TS${diagnostic.code}`);
}

