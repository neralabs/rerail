/**
 * The ECMA rename for upstream-labelled rename variants: a generator writes a renamed copy of each upstream test unit
 * (`writeRenameVariants`, scripts/pack-data/eslint-expected.mjs) and upstream lints the copy like any unit, so a port
 * that matches the tests' own names (`foo`, `MyClass`, `bar`) instead of the construct the rule is about fails on the
 * copy. Reads the source with TypeScript's parser as TypeScript would by its extension (JavaScript with JSX, JSX, TS
 * or TSX). Node's type stripping runs this file from the generators, so it imports nothing but `typescript` and uses
 * erasable syntax only.
 *
 * - Identifiers only: every distinct identifier text (declarations, references, property names, shorthand, types,
 *   labels, user JSX tags and attributes, private names after their `#`) and every name a `/* global *\/`,
 *   `/* globals *\/` or `/* exported *\/` comment lists is renamed through one map per file, applied to every
 *   occurrence. Literals are kept as they are: string and template literal contents, numbers, regular expressions,
 *   JSX text and other comments. A rule reads them (the digits prefer-numeric-literals and radix check, the URL
 *   no-script-url and Next's rules check, the font names no-page-custom-font checks), so renaming them changed what
 *   upstream reports, not how a port finds it. A name that is also a word inside a kept literal (`o["name"]`,
 *   `hasOwnProperty("name")`, `{ a: 1, "a": 2 }`) stays, so every name-equality relation between code and literals holds.
 * - Same length and shape: each renamed name keeps its UTF-16 length; only its ASCII letters change, each to a letter
 *   of the same case (digits, `_`, `$` and non-ASCII characters stay put, and so does the `I` of a Hungarian `IName`),
 *   so a variant's findings sit where the original's do and naming-convention rules judge the same shape.
 * - Order-preserving: the map is a relabelling of the trie of the file's names, letters compared case-insensitively.
 *   Each name's i-th letter becomes a function of its first i letters (case folded), strictly increasing among the
 *   letters that follow the same prefix, and a name that stays keeps every letter. So the map is a bijection (every
 *   binding, shadowing and name-equality relation in the file holds without modelling scopes), and any two names of
 *   the file (renamed or staying, literal words included) compare the same way after renaming as before: by code units
 *   (sort-keys, sort-vars and sort-imports at their defaults), ignoring case, by locale and naturally (digit runs stay).
 * - Protected, the APIs a rule is about rather than the tests' own names: keywords and contextual keywords, the ES
 *   standard library (globals and every member declared in TypeScript's lib.es*.d.ts), the names `HOST_NAMES` lists (a
 *   few browser and Node globals, Node's deprecated `GLOBAL`, `root` and `COUNTER_*`, console methods, React's component
 *   API and lifecycle, Next's data-fetching exports, Node's callback conventions, the test runners' globals and
 *   modifiers: `vi`, `jest`, `only`, `skip`, `todo`), any
 *   hook-shaped name (`use` and an uppercase letter) and any `…Sync` name; every member name chained on one of
 *   `MEMBER_ROOTS` (`process.exit`, `ReactDOM.findDOMNode`, `vi.fn`, `expect(x).toBe`, `Intl.v8BreakIterator`) and every
 *   key destructured from one (`let { env: { NODE_REPL_HISTORY_FILE } } = process`); and what a package exports:
 *   the names an `import` or a destructuring `require` takes from a package (`import { Head } from "next/document"`,
 *   `const { TextDecoder } = require("util")`), the members read on a package's binding (`fs.access`) and the props
 *   of a package's component (`<Script strategy=…>`). The rule a variant then loses every finding of is one that
 *   matches the tests' own names, and its `holds-under-rename` fails. Other names of a host API (`setTimeout`'s
 *   callback's parameters, a DOM method on a local) are renamed like the tests' own. Also kept: intrinsic JSX tags
 *   and the attributes on intrinsic elements, both parts of a namespaced JSX name (`xlink:href`), meta-property names (`import.meta`,
 *   `new.target`) and any name with an occurrence that cannot be rewritten in place (unicode escapes). No name is
 *   renamed to a protected name, to one of a kept shape or (when longer than two letters) to a word in the file's
 *   comments; a name that cannot avoid them under any of the tried relabellings stays.
 */
import { createRequire } from "node:module";
import { readdirSync, readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import ts from "typescript";

/** The test runners' globals. */
const TEST_GLOBALS = ["describe", "it", "test", "expect", "suite", "bench", "vi", "jest", "xit", "xtest", "xdescribe", "fit", "fdescribe"];

/**
 * Host objects whose members are their API: the member names of a chain of accesses and calls on one are kept
 * (`process.exit`, `ReactDOM.findDOMNode`, `vi.fn`, `jest.mock`, `it.only.each`, `expect(x).not.toBe`).
 */
const MEMBER_ROOTS = new Set([...TEST_GLOBALS, "process", "console", "React", "ReactDOM", "window", "document", "navigator", "location", "globalThis", "self", "global", "Buffer", "module", "exports", "require", "Intl"]);

/** The root identifier of a chain of member accesses and calls (`a` of `a.b(c).d`), if it has one. */
function chainRoot(expression: ts.Expression): ts.Identifier | undefined {
  let root = expression;
  while (ts.isPropertyAccessExpression(root) || ts.isCallExpression(root) || ts.isElementAccessExpression(root) || ts.isNonNullExpression(root)) root = root.expression;
  return ts.isIdentifier(root) ? root : undefined;
}

/** Whether `specifier` names a package (or a Node builtin), not a file of the test's own. */
const isPackage = (specifier: string): boolean => !specifier.startsWith(".") && !specifier.startsWith("/");

/** The package a `require("…")` (or `process.getBuiltinModule("…")`, `await import("…")`) call loads, if `node` is one. */
function requiredPackage(node: ts.Node | undefined): string | undefined {
  while (node !== undefined && (ts.isAwaitExpression(node) || ts.isParenthesizedExpression(node))) node = node.expression;
  if (node === undefined || !ts.isCallExpression(node)) return undefined;
  const callee = node.expression;
  const loads =
    callee.kind === ts.SyntaxKind.ImportKeyword ||
    (ts.isIdentifier(callee) && callee.text === "require") ||
    (ts.isPropertyAccessExpression(callee) && callee.name.text === "getBuiltinModule");
  const argument = node.arguments[0];
  return loads && argument !== undefined && ts.isStringLiteralLike(argument) && isPackage(argument.text) ? argument.text : undefined;
}

/**
 * The names a package defines in `file`: what its imports and destructuring requires take from a package (kept), and
 * the local bindings of a package's module or default export (`import fs from "fs"`, `const fs = require("fs")`,
 * `import * as React from "react"`), whose members and component props are the package's.
 */
function packageNames(file: ts.SourceFile): { exported: Set<string>; bindings: Set<string> } {
  const exported = new Set<string>();
  const bindings = new Set<string>();
  const visit = (node: ts.Node): void => {
    if (ts.isImportDeclaration(node) && ts.isStringLiteralLike(node.moduleSpecifier) && isPackage(node.moduleSpecifier.text) && node.importClause !== undefined) {
      const clause = node.importClause;
      if (clause.name !== undefined) bindings.add(clause.name.text);
      const named = clause.namedBindings;
      if (named !== undefined && ts.isNamespaceImport(named)) bindings.add(named.name.text);
      if (named !== undefined && ts.isNamedImports(named)) for (const element of named.elements) exported.add((element.propertyName ?? element.name).text);
    }
    // Destructuring a host object takes its API: `let { env: { NODE_REPL_HISTORY_FILE } } = process`.
    if (ts.isVariableDeclaration(node) && ts.isObjectBindingPattern(node.name) && node.initializer !== undefined) {
      const root = chainRoot(node.initializer);
      if (root !== undefined && MEMBER_ROOTS.has(root.text)) for (const name of patternKeys(node.name)) exported.add(name);
    }
    if (ts.isVariableDeclaration(node) && requiredPackage(node.initializer) !== undefined) {
      if (ts.isIdentifier(node.name)) bindings.add(node.name.text);
      else if (ts.isObjectBindingPattern(node.name)) {
        for (const element of node.name.elements) {
          const name = element.propertyName ?? element.name;
          if (ts.isIdentifier(name)) exported.add(name.text);
        }
      }
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  return { exported, bindings };
}

/** The property names an object binding pattern reads, nested patterns' included (`env` and `X` of `{ env: { X } }`). */
function patternKeys(pattern: ts.ObjectBindingPattern): string[] {
  return pattern.elements.flatMap((element) => {
    const key = element.propertyName ?? element.name;
    const own = ts.isIdentifier(key) ? [key.text] : [];
    return ts.isObjectBindingPattern(element.name) ? [...own, ...patternKeys(element.name)] : own;
  });
}

/** Whether `node` is a member name chained on a host object or on a package's binding, or a prop of a package's component. */
function isApiName(node: ts.Identifier | ts.PrivateIdentifier, bindings: ReadonlySet<string>): boolean {
  const parent = node.parent;
  if (ts.isPropertyAccessExpression(parent) && parent.name === node) {
    const root = chainRoot(parent.expression);
    return root !== undefined && (MEMBER_ROOTS.has(root.text) || bindings.has(root.text));
  }
  if (ts.isJsxAttribute(parent) && parent.name === node) {
    const tag = parent.parent.parent.tagName;
    const root = ts.isIdentifier(tag) ? tag : ts.isPropertyAccessExpression(tag) ? chainRoot(tag) : undefined;
    return root !== undefined && bindings.has(root.text);
  }
  return false;
}

/** The host, module-system, React and test-runner names kept (not exhaustive: any name missing here is renamed). */
const HOST_NAMES = [
  "window", "document", "navigator", "location", "globalThis", "self", "console", "process", "Buffer", "global", "require", "module", "exports",
  "define", "__dirname", "__filename", "GLOBAL", "root", "COUNTER_HTTP_CLIENT_REQUEST", "COUNTER_HTTP_CLIENT_RESPONSE",
  "COUNTER_HTTP_SERVER_REQUEST", "COUNTER_HTTP_SERVER_RESPONSE", "COUNTER_NET_SERVER_CONNECTION", "COUNTER_NET_SERVER_CONNECTION_CLOSE", "setTimeout", "clearTimeout", "setInterval", "clearInterval", "setImmediate", "clearImmediate",
  "requestAnimationFrame", "fetch", "alert", "confirm", "prompt", "localStorage", "sessionStorage", "addEventListener", "removeEventListener",
  "log", "warn", "error", "info", "debug", "trace", "time", "timeEnd", "dir", "table", "assert",
  "React", "ReactDOM", "Component", "PureComponent", "Fragment", "createElement", "props", "state", "setState", "render", "children", "key",
  "ref", "className", "createReactClass", "createClass", "findDOMNode", "isMounted", "replaceState", "forceUpdate", "getInitialState",
  "getDefaultProps", "getChildContext", "displayName", "propTypes", "defaultProps", "contextTypes", "childContextTypes", "contextType",
  "componentDidMount", "componentDidUpdate", "componentWillUnmount", "componentWillMount", "componentWillReceiveProps", "componentWillUpdate",
  "shouldComponentUpdate", "getSnapshotBeforeUpdate", "getDerivedStateFromProps", "getDerivedStateFromError", "componentDidCatch",
  "UNSAFE_componentWillMount", "UNSAFE_componentWillReceiveProps", "UNSAFE_componentWillUpdate", "createContext", "Provider", "Consumer", "value",
  "getStaticProps", "getStaticPaths", "getServerSideProps", "getInitialProps",
  "callback", "cb", "next", "done", "err",
  "jQuery", "$", "_", "angular",
  ...TEST_GLOBALS, "beforeEach", "afterEach", "beforeAll", "afterAll", "vitest", "only", "skip", "todo", "each", "concurrent", "sequential",
  "fails", "skipIf", "runIf",
  "arguments", "eval", "undefined", "NaN", "Infinity", "constructor", "prototype", "__proto__", "this", "super",
];

/**
 * Name shapes a rule reads, so a name of one is kept and none is made: a React hook's (`useState`, a custom `useThing`)
 * and a synchronous Node API's (`readFileSync`, `fooSync`).
 */
const API_SHAPE = /^use[A-Z]|[a-z0-9]Sync$/;

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

/** Whether `name` (without a private name's `#`) is never renamed: a protected name or one of a kept shape (`API_SHAPE`). */
function isProtected(name: string): boolean {
  return protectedNames().has(name) || API_SHAPE.test(name);
}

export interface RenameResult {
  source: string;
  /** Original name (identifier text, `#` kept for private names) to its replacement; names that stay are absent. */
  renames: Map<string, string>;
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

const LOWER_A = 97;
const LOWER_Z = 122;
const isAsciiLetter = (char: string): boolean => /^[A-Za-z]$/.test(char);
/** A character's trie key: an ASCII letter case folded, anything else itself. */
const fold = (char: string): string => (isAsciiLetter(char) ? char.toLowerCase() : char);

/** A trie of the file's names over case-folded characters; `fixed` children keep their letter. */
interface TrieNode {
  children: Map<string, TrieNode>;
  fixed: Set<string>;
  /** Each child letter's new letter (lower case), once labelled. */
  labels: Map<string, string>;
}

const trieNode = (): TrieNode => ({ children: new Map(), fixed: new Set(), labels: new Map() });

/** Adds `name`; `fixedPrefix` of its characters keep their letter (all of a staying name, the `I` of `IName`). */
function insert(root: TrieNode, name: string, fixedPrefix: number): void {
  let node = root;
  for (let index = 0; index < name.length; index += 1) {
    const key = fold(name[index]!);
    if (index < fixedPrefix) node.fixed.add(key);
    let child = node.children.get(key);
    if (child === undefined) node.children.set(key, (child = trieNode()));
    node = child;
  }
}

/**
 * Labels every node's child letters: strictly increasing in the original letters' order, a fixed letter keeping its
 * own, so each run of free letters between two fixed ones (or the alphabet's ends) takes a random sorted subset of the
 * letters strictly between them (there are always enough: the originals are such a subset).
 */
function label(node: TrieNode, next: () => number): void {
  const letters = [...node.children.keys()].filter(key => key.length === 1 && key >= "a" && key <= "z").sort();
  const bounds = [LOWER_A - 1, ...letters.filter(letter => node.fixed.has(letter)).map(letter => letter.charCodeAt(0)), LOWER_Z + 1];
  for (const letter of letters) if (node.fixed.has(letter)) node.labels.set(letter, letter);
  for (let index = 0; index + 1 < bounds.length; index += 1) {
    const low = bounds[index]!;
    const high = bounds[index + 1]!;
    const free = letters.filter(letter => !node.fixed.has(letter) && letter.charCodeAt(0) > low && letter.charCodeAt(0) < high);
    if (free.length === 0) continue;
    const slots = Array.from({ length: high - low - 1 }, (_, offset) => low + 1 + offset);
    let chosen: number[] = [];
    // Prefer a choice that changes something: a run with room to spare is redrawn a few times if it drew the originals.
    for (let draw = 0; draw < 4; draw += 1) {
      const pool = [...slots];
      for (let pick = 0; pick < free.length; pick += 1) {
        const other = pick + (next() % (pool.length - pick));
        [pool[pick], pool[other]] = [pool[other]!, pool[pick]!];
      }
      chosen = pool.slice(0, free.length).sort((left, right) => left - right);
      if (chosen.some((code, position) => code !== free[position]!.charCodeAt(0))) break;
    }
    free.forEach((letter, position) => node.labels.set(letter, String.fromCharCode(chosen[position]!)));
  }
  for (const child of node.children.values()) label(child, next);
}

/** `name` through the labelled trie: each ASCII letter its node's label in the letter's own case. */
function relabel(root: TrieNode, name: string): string {
  let node = root;
  let result = "";
  for (const char of name) {
    const key = fold(char);
    const letter = node.labels.get(key);
    result += letter === undefined ? char : char === key ? letter : letter.toUpperCase();
    node = node.children.get(key)!;
  }
  return result;
}

function isIntrinsicTag(name: ts.JsxTagNameExpression): boolean {
  return ts.isIdentifier(name) && /^[a-z]/.test(name.text) && !name.text.includes(".");
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

const WORD = /[A-Za-z_$][\w$]*/g;

/** ESLint's block-comment directives that list variable names: `/* global a, b: writable *\/`, `globals`, `exported`. */
const NAME_LIST_DIRECTIVE = /^\s*(?:globals?|exported)(?=\s)/u;

/** How TypeScript reads the file, as it would by extension: `.js`, `.mjs` and `.cjs` as JavaScript (JSX allowed). */
function scriptKind(fileName: string): ts.ScriptKind {
  if (/\.tsx$/.test(fileName)) return ts.ScriptKind.TSX;
  if (/\.jsx$/.test(fileName)) return ts.ScriptKind.JSX;
  if (/\.[cm]?js$/.test(fileName)) return ts.ScriptKind.JS;
  return ts.ScriptKind.TS;
}

/** Relabellings tried before a name whose every image is unusable is kept as it is. */
const ATTEMPTS = 16;

/** Renames `source`'s identifiers (see module doc). `fileName` picks how it is parsed. */
export function renameSource(source: string, fileName: string): RenameResult {
  const file = ts.createSourceFile(fileName, source, ts.ScriptTarget.ESNext, true, scriptKind(fileName));
  const occurrences = new Map<string, Occurrence[]>();
  const pinned = new Set<string>();
  const literalWords = new Set<string>();
  const comments = new Map<number, ts.CommentRange>();
  const occur = (text: string, occurrence: Occurrence): void => {
    const bucket = occurrences.get(text) ?? [];
    bucket.push(occurrence);
    occurrences.set(text, bucket);
  };
  const collectComments = (node: ts.Node): void => {
    for (const range of [...(ts.getLeadingCommentRanges(source, node.pos) ?? []), ...(ts.getTrailingCommentRanges(source, node.end) ?? [])]) comments.set(range.pos, range);
  };
  const packages = packageNames(file);
  for (const name of packages.exported) pinned.add(name);
  const visit = (node: ts.Node): void => {
    collectComments(node);
    if (ts.isIdentifier(node) || ts.isPrivateIdentifier(node)) {
      const text = node.text;
      if (text === "") return;
      const start = node.getStart(file);
      const parent = node.parent;
      const intrinsicTag = (ts.isJsxOpeningElement(parent) || ts.isJsxSelfClosingElement(parent) || ts.isJsxClosingElement(parent)) && parent.tagName === node && isIntrinsicTag(parent.tagName);
      const intrinsicAttribute = ts.isJsxAttribute(parent) && parent.name === node && isIntrinsicTag(parent.parent.parent.tagName);
      const namespaced = ts.isJsxNamespacedName(parent);
      const keywordLike = ts.isIdentifier(node) && ts.identifierToKeywordKind(node) !== undefined;
      const metaProperty = ts.isMetaProperty(parent) && parent.name === node;
      const fixed = intrinsicTag || intrinsicAttribute || namespaced || keywordLike || metaProperty || isApiName(node, packages.bindings);
      if (source.slice(start, node.end) !== text || fixed) pinned.add(text);
      occur(text, { start, offset: ts.isPrivateIdentifier(node) ? 1 : 0 });
      return;
    }
    const content = literalContent(node, file);
    if (content !== undefined) {
      for (const word of source.slice(...content).match(WORD) ?? []) literalWords.add(word);
      return;
    }
    ts.forEachChild(node, visit);
  };
  visit(file);
  collectComments(file.endOfFileToken);

  // Names a `/* global */`-style comment lists are occurrences too; every other comment word is only avoided.
  const commentWords = new Set<string>();
  for (const range of comments.values()) {
    const text = source.slice(range.pos, range.end);
    const directive = range.kind === ts.SyntaxKind.MultiLineCommentTrivia ? NAME_LIST_DIRECTIVE.exec(text.slice(2, -2)) : null;
    if (directive === null) {
      for (const word of text.match(WORD) ?? []) commentWords.add(word);
      continue;
    }
    const bodyStart = range.pos + 2 + directive[0].length;
    const body = source.slice(bodyStart, range.end - 2).split(/\s--\s/u)[0]!;
    // ESLint's parseStringConfig: `name` or `name:value` items, separated by commas or whitespace.
    for (const item of body.matchAll(/([^\s,:]+)(?:\s*:\s*[^\s,]*)?/gu)) {
      const name = item[1]!;
      if (/^[A-Za-z_$][\w$]*$/u.test(name)) occur(name, { start: bodyStart + item.index!, offset: 0 });
    }
  }

  const bareName = (name: string): string => (name.startsWith("#") ? name.slice(1) : name);
  const hungarian = (name: string): boolean => /^I[A-Z]/.test(name);
  const staying = new Set<string>(literalWords);
  const renamable = new Set<string>();
  for (const name of occurrences.keys()) {
    const bare = bareName(name);
    if (pinned.has(name) || literalWords.has(bare) || isProtected(bare) || !/[A-Za-z]/.test(bare.slice(hungarian(bare) ? 1 : 0))) staying.add(bare);
    else renamable.add(bare);
  }
  // A name kept for one reason (`#a` beside a pinned `a`) is kept everywhere: the trie is keyed by the bare name.
  for (const name of staying) renamable.delete(name);

  // The trie keeps every image apart from the file's other names; only names the file does not hold need checking.
  const unusable = (from: string, to: string): boolean => to !== from && (isProtected(to) || (to.length > 2 && commentWords.has(to)));
  let images = new Map<string, string>();
  for (let attempt = 0; ; attempt += 1) {
    const root = trieNode();
    for (const name of staying) insert(root, name, name.length);
    for (const name of renamable) insert(root, name, hungarian(name) ? 1 : 0);
    label(root, xorshift(fnv1a(`${source}\u0000${attempt}`)));
    images = new Map([...renamable].map(name => [name, relabel(root, name)]));
    const clashes = [...images].filter(([from, to]) => unusable(from, to)).map(([from]) => from);
    if (clashes.length === 0) break;
    // After ATTEMPTS relabellings, the names that still clash stay as they are, and the rest are relabelled around them.
    if (attempt >= ATTEMPTS) {
      for (const name of clashes) {
        renamable.delete(name);
        staying.add(name);
      }
    }
  }

  const renames = new Map<string, string>();
  const edits: Array<{ start: number; text: string }> = [];
  for (const [name, list] of occurrences) {
    const bare = bareName(name);
    const image = images.get(bare);
    if (image === undefined || image === bare) continue;
    renames.set(name, name.startsWith("#") ? `#${image}` : image);
    for (const { start, offset } of list) edits.push({ start: start + offset, text: image });
  }
  edits.sort((left, right) => left.start - right.start);
  let result = "";
  let cursor = 0;
  for (const edit of edits) {
    result += source.slice(cursor, edit.start) + edit.text;
    cursor = edit.start + edit.text.length;
  }
  result += source.slice(cursor);
  return { source: result, renames };
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

/** `source` renamed, or why no variant is written: no change, or a changed length or TypeScript parse. */
export function renameVariant(source: string, fileName: string): { source: string } | { problem: string } {
  const renamed = renameSource(source, fileName);
  if (renamed.source === source) return { problem: "nothing to rename" };
  if (renamed.source.length !== source.length) return { problem: "length changed" };
  const before = syntaxDiagnostics(source, fileName).join(" ");
  const after = syntaxDiagnostics(renamed.source, fileName).join(" ");
  if (before !== after) return { problem: `TypeScript syntax diagnostics changed: [${before}] -> [${after}]` };
  return { source: renamed.source };
}
