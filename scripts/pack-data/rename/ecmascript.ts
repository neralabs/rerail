/**
 * The ECMA rename for upstream-labelled rename variants: a generator writes a renamed copy of each upstream test unit
 * (`writeRenameVariants`, scripts/pack-data/eslint-expected.mjs) and upstream lints the copy like any unit, so a port
 * that matches the tests' own names and values (`foo`, `MyClass`, `"bar"`, `42`) instead of the construct the rule is
 * about fails on the copy. Reads the source with TypeScript's parser as TypeScript would by its extension (JavaScript
 * with JSX, JSX, TS or TSX). Node's type stripping runs this file from the generators, so it imports nothing but
 * `typescript` and uses erasable syntax only.
 *
 * - Same length: each renamed word keeps its UTF-16 length and character shape (upper stays upper, lower stays
 *   lower, digits stay digits, `_`/`$`/non-ASCII stay put, a Hungarian `I` before an uppercase letter stays), so a
 *   variant's findings sit where the original's do and naming-convention rules judge the same shape.
 * - One map per file: every distinct identifier text and every word inside a string or template literal is renamed
 *   to one opaque word, applied to every occurrence (declarations, references, property names, shorthand, types,
 *   labels, user JSX tags, `o["name"]`, `hasOwnProperty("name")`). The map is a bijection, so every binding,
 *   shadowing and name-equality relation in the file is kept without modelling scopes.
 * - Numbers: the digits 1-9 of decimal and hexadecimal literals (not their exponents) go through one permutation
 *   per file, so equal numbers stay equal and zero stays zero. Legacy octal, `0o` and `0b` literals stay.
 * - Protected: keywords and contextual keywords, the ES standard library (globals and every member declared in
 *   TypeScript's lib.es*.d.ts) and a curated host list (DOM/Node/React/module names); intrinsic JSX tags and the
 *   attributes on intrinsic elements; any name with an occurrence that cannot be rewritten in place (unicode
 *   escapes); module specifiers, import attributes, directives (`"use strict"`) and meta-property names (`import.meta`,
 *   `new.target`); escape sequences inside literals. Comments, regular expressions and JSX text are never touched.
 */
import { createRequire } from "node:module";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import ts from "typescript";

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
function standardLibraryNames(): ReadonlySet<string> {
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

let globalNames: ReadonlySet<string> | undefined;

/** Keywords (reserved and contextual), the standard library and the host list: never renamed. */
export function protectedNames(): ReadonlySet<string> {
  if (globalNames !== undefined) return globalNames;
  const names = new Set<string>([...standardLibraryNames(), ...HOST_NAMES, ...CONTEXTUAL_KEYWORDS]);
  for (let kind = ts.SyntaxKind.FirstKeyword; kind <= ts.SyntaxKind.LastKeyword; kind += 1) {
    const text = ts.tokenToString(kind);
    if (text !== undefined) names.add(text);
  }
  globalNames = names;
  return names;
}

export interface RenameResult {
  source: string;
  /** Original word (identifier text, `#` kept for private names, or a word in a literal) to its opaque replacement. */
  renames: Map<string, string>;
  /** Words that would be renamed but have no free same-shape replacement (e.g. every one-letter name is taken). */
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

/** A deterministic permutation of the digits 1-9 (0 maps to itself), seeded by `seed`. */
function digitPermutation(seed: string): string {
  const next = xorshift(fnv1a(seed));
  const digits = [..."123456789"];
  for (let index = digits.length - 1; index > 0; index -= 1) {
    const other = next() % (index + 1);
    [digits[index], digits[other]] = [digits[other]!, digits[index]!];
  }
  return `0${digits.join("")}`;
}

function isIntrinsicTag(name: ts.JsxTagNameExpression): boolean {
  return ts.isIdentifier(name) && /^[a-z]/.test(name.text) && !name.text.includes(".");
}

/** A literal whose text the program reads as more than a value: a module specifier, an import attribute, a directive. */
function isFixedLiteral(node: ts.Node): boolean {
  const parent = node.parent;
  if ((ts.isImportDeclaration(parent) || ts.isExportDeclaration(parent)) && parent.moduleSpecifier === node) return true;
  if (ts.isExternalModuleReference(parent) || ts.isImportAttribute(parent) || ts.isExpressionStatement(parent)) return true;
  if (ts.isModuleDeclaration(parent) && parent.name === node) return true;
  if (ts.isLiteralTypeNode(parent) && ts.isImportTypeNode(parent.parent)) return true;
  if (ts.isCallExpression(parent) && parent.arguments[0] === node) {
    const callee = parent.expression;
    return callee.kind === ts.SyntaxKind.ImportKeyword || (ts.isIdentifier(callee) && callee.text === "require");
  }
  return false;
}

/** The raw content of a string or template literal token: [start, end) offsets inside its delimiters. */
function literalContent(node: ts.Node, file: ts.SourceFile): [number, number] | undefined {
  const start = node.getStart(file);
  switch (node.kind) {
    case ts.SyntaxKind.StringLiteral:
    case ts.SyntaxKind.NoSubstitutionTemplateLiteral:
    case ts.SyntaxKind.TemplateTail:
      return [start + 1, node.end - 1];
    case ts.SyntaxKind.TemplateHead:
    case ts.SyntaxKind.TemplateMiddle:
      return [start + 1, node.end - 2];
    default:
      return undefined;
  }
}

/** The length of the escape sequence at `index` (a backslash) in a literal's raw text. */
function escapeLength(raw: string, index: number): number {
  const next = raw[index + 1];
  if (next === "u") return raw[index + 2] === "{" ? Math.max(3, raw.indexOf("}", index) - index + 1) : 6;
  if (next === "x") return 4;
  if (next === "\r" && raw[index + 2] === "\n") return 3;
  return 2;
}

const WORD_START = /[A-Za-z_$]/;
const WORD_PART = /[A-Za-z0-9_$]/;

/** How TypeScript reads the file, as it would by extension: `.js`, `.mjs` and `.cjs` as JavaScript (JSX allowed). */
function scriptKind(fileName: string): ts.ScriptKind {
  if (/\.tsx$/.test(fileName)) return ts.ScriptKind.TSX;
  if (/\.jsx$/.test(fileName)) return ts.ScriptKind.JSX;
  if (/\.[cm]?js$/.test(fileName)) return ts.ScriptKind.JS;
  return ts.ScriptKind.TS;
}

/** Renames `source`'s identifiers, literal words and numbers (see module doc). `fileName` picks how it is parsed. */
export function renameSource(source: string, fileName: string): RenameResult {
  const protection = protectedNames();
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.ESNext, true, scriptKind(fileName));
  const occurrences = new Map<string, Occurrence[]>();
  const pinned = new Set<string>();
  const edits: Array<{ start: number; text: string }> = [];
  const permutation = digitPermutation(source);
  const occur = (text: string, occurrence: Occurrence): void => {
    const bucket = occurrences.get(text) ?? [];
    bucket.push(occurrence);
    occurrences.set(text, bucket);
  };
  const renumber = (start: number, digits: string): void => {
    edits.push({ start, text: digits.replace(/[0-9]/g, digit => permutation[Number(digit)]!) });
  };
  const scanLiteral = (start: number, end: number): void => {
    const raw = source.slice(start, end);
    let index = 0;
    while (index < raw.length) {
      const char = raw[index]!;
      if (char === "\\") index += escapeLength(raw, index);
      else if (WORD_START.test(char)) {
        let wordEnd = index + 1;
        while (wordEnd < raw.length && WORD_PART.test(raw[wordEnd]!)) wordEnd += 1;
        occur(raw.slice(index, wordEnd), { start: start + index, offset: 0 });
        index = wordEnd;
      } else if (/[0-9]/.test(char)) {
        let digitsEnd = index + 1;
        while (digitsEnd < raw.length && /[0-9]/.test(raw[digitsEnd]!)) digitsEnd += 1;
        renumber(start + index, raw.slice(index, digitsEnd));
        index = digitsEnd;
      } else index += 1;
    }
  };
  const scanNumber = (node: ts.Node): void => {
    const start = node.getStart(file);
    const raw = source.slice(start, node.end);
    if (/^0[0-9]|^0[bBoO]/.test(raw)) return;
    const hex = /^0[xX]/.test(raw);
    const mantissa = hex ? raw.slice(2) : raw.split(/[eE]/)[0]!;
    renumber(start + (hex ? 2 : 0), mantissa);
  };
  const visit = (node: ts.Node): void => {
    if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node)) {
      const text = node.text;
      if (text === "") return;
      const start = node.getStart(file);
      const parent = node.parent;
      const intrinsicTag = (ts.isJsxOpeningElement(parent) || ts.isJsxSelfClosingElement(parent) || ts.isJsxClosingElement(parent)) && parent.tagName === node && isIntrinsicTag(parent.tagName);
      const intrinsicAttribute = ts.isJsxAttribute(parent) && parent.name === node && isIntrinsicTag(parent.parent.parent.tagName);
      const keywordLike = ts.isIdentifier(node) && ts.identifierToKeywordKind(node) !== undefined;
      const metaProperty = ts.isMetaProperty(parent) && parent.name === node;
      if (source.slice(start, node.end) !== text || intrinsicTag || intrinsicAttribute || keywordLike || metaProperty) pinned.add(text);
      occur(text, { start, offset: ts.isPrivateIdentifier(node) ? 1 : 0 });
      return;
    }
    if (ts.isNumericLiteral(node) || ts.isBigIntLiteral(node)) {
      scanNumber(node);
      return;
    }
    const content = literalContent(node, file);
    if (content !== undefined) {
      if (!isFixedLiteral(node)) scanLiteral(...content);
      return;
    }
    ts.forEachChild(node, visit);
  };
  visit(file);

  const bareName = (name: string): string => (name.startsWith("#") ? name.slice(1) : name);
  const renamable = [...occurrences.keys()]
    .filter(name => !pinned.has(name) && !protection.has(bareName(name)) && /[A-Za-z]/.test(bareName(name).slice(/^I[A-Z]/.test(bareName(name)) ? 1 : 0)))
    .sort((left, right) => (left === right ? 0 : left < right ? -1 : 1));
  const renamableSet = new Set(renamable);
  // Targets may reuse another renamed word (a permutation keeps the map a bijection), never a word that stays.
  const staying = new Set([...occurrences.keys()].filter(text => !renamableSet.has(text)).map(bareName));
  // Words in comments are avoided too, except for one- and two-letter names, whose shapes run out.
  const words = new Set((source.match(/[A-Za-z_$][\w$]*/g) ?? []).filter(word => !occurrences.has(word)));
  const assigned = new Set<string>();
  const renames = new Map<string, string>();
  const exhausted: string[] = [];
  for (const name of renamable) {
    const bare = bareName(name);
    let replacement: string | undefined;
    for (let attempt = 0; attempt < 500 && replacement === undefined; attempt += 1) {
      const next = candidate(bare, attempt);
      const blocked = staying.has(next) || protection.has(next) || assigned.has(next) || (bare.length > 2 && words.has(next));
      if (next !== bare && !blocked) replacement = next;
    }
    if (replacement === undefined) {
      exhausted.push(name);
      continue;
    }
    assigned.add(replacement);
    renames.set(name, name.startsWith("#") ? `#${replacement}` : replacement);
  }

  for (const [name, replacement] of renames) {
    for (const { start, offset } of occurrences.get(name)!) edits.push({ start: start + offset, text: replacement.slice(offset) });
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

/** TypeScript's syntax diagnostics for `source` (offset and code), to check a rename did not change how it parses. */
export function syntaxDiagnostics(source: string, fileName: string): string[] {
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.ESNext, true, scriptKind(fileName));
  const program = ts.createProgram({
    rootNames: [fileName],
    options: { noEmit: true, noLib: true, types: [], jsx: ts.JsxEmit.Preserve, allowJs: true },
    host: {
      ...ts.createCompilerHost({}),
      getSourceFile: name => (name === fileName ? file : undefined),
      fileExists: name => name === fileName,
      readFile: name => (name === fileName ? source : undefined),
    },
  });
  return program.getSyntacticDiagnostics(file).map(diagnostic => `${diagnostic.start ?? -1}:TS${diagnostic.code}`);
}

/**
 * `source` renamed, or why no variant is written: no change, a changed length or TypeScript parse, or a word left
 * without a replacement.
 */
export function renameVariant(source: string, fileName: string): { source: string } | { problem: string } {
  const renamed = renameSource(source, fileName);
  if (renamed.source === source) return { problem: "nothing to rename" };
  if (renamed.source.length !== source.length) return { problem: "length changed" };
  if (renamed.exhausted.length > 0) return { problem: `no free same-shape word for ${renamed.exhausted.join(", ")}` };
  const before = syntaxDiagnostics(source, fileName).join(" ");
  const after = syntaxDiagnostics(renamed.source, fileName).join(" ");
  if (before !== after) return { problem: `TypeScript syntax diagnostics changed: [${before}] -> [${after}]` };
  return { source: renamed.source };
}
