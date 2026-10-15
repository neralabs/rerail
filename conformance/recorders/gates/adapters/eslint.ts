import { createHash } from "node:crypto";
import { existsSync, readdirSync, readFileSync } from "node:fs";
import { appendFile, mkdir, mkdtemp, readFile, realpath, rename, rm, symlink } from "node:fs/promises";
import { createRequire } from "node:module";
import { dirname, extname, join, matchesGlob, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

import ts from "typescript";

import { runChecked, type CommandRunner } from "../../../gates/process.js";
import { eslintSpan, eslintSpanParts, messageSpan } from "../../../gates/span-units/eslint.js";
import type { Check, EslintUpstream, FixtureTextSource, PackAdapter, PackManifest, Span, UpstreamCase, UpstreamFile, UpstreamRule } from "../../../gates/types.js";
import {
  commentVocabulary, keywords, literalNames, literalRegexes, parserVerdicts, sourceWords, words,
  type BabelParser, type DocumentationNode, type TypeScript,
} from "../languages/ecmascript.js";
import { checkoutUpstream, filesUnder, npmRuntime, treeDigest } from "../runtimes.js";
import type { MessagePosition } from "./eslint-upstream.js";

/**
 * The ESLint adapter, for ESLint's own rules and any ESLint plugin's, as the pack manifest's `eslint` block names
 * them. Expected answers come from running the pinned ESLint itself, in the runtime of runtimes/<runtime>:
 * every case of every upstream test file of the rule, loaded with upstream's own RuleTester, is linted with the rule
 * as the first approved configuration sets it (its own options aside), in the language the case sets (parser, parser
 * options, ecmaVersion, sourceType) and without the globals and settings it or its RuleTester declares, which the
 * approved configuration does not, at the file upstream names for it. Corpus files are linted as upstream's approved
 * configuration lints a project's files.
 */

const DRIVER = fileURLToPath(new URL("./eslint-driver.mjs", import.meta.url));

/** ESLint publishes eslint:recommended in @eslint/js, built from this directory of its repository. */
const ESLINT_JS = "packages/js";

interface RuleFacts {
  deprecated: string | null;
  requiresTypeChecking: boolean;
  extendsBaseRule: string | null;
  recommended: { config: string; options: unknown[] } | null;
}
interface Facts {
  eslint: string;
  node: string;
  plugin: string | null;
  configs: { package: string; version: string };
  rules: Record<string, RuleFacts | null>;
  extensions: string[];
  /** The module files of the parsers the approved configurations lint with. */
  parsers: string[];
  directives: string[];
}
interface DriverCase {
  fixture: string;
  variant: string;
  path: string;
  code: string;
  /** Linted as an ECMAScript script (`sourceType: "script"`); absent for a case upstream did not lint. */
  script?: boolean;
  /** The file type upstream's parser read the case as, when its path's is another language's. */
  parsedAs?: string;
  skipped: string | null;
  findings: MessagePosition[];
  markup: MessagePosition[] | null;
}
interface Tests {
  files: { file: string; runs: number; error: string | null }[];
  texts: { code: string; jsx: boolean }[];
  configurations: string[];
  testFiles: Record<string, string[]>;
  cases: Record<string, DriverCase[]>;
}
interface Linted { excluded: string | null; findings: Record<string, MessagePosition[]> }

/** A rule name as the plugin's rule map keys it: `@typescript-eslint/no-explicit-any` and `no-explicit-any` are the same rule. */
export function bareRuleName(plugin: EslintUpstream["plugin"], name: string): string {
  return plugin !== null && name.startsWith(`${plugin.namespace}/`) ? name.slice(plugin.namespace.length + 1) : name;
}

/** The manifest's implementation path of a rule: `{rule}` is its name, `{Rule}` its name in PascalCase (`RulesOfHooks`). */
export function implementationPath(pattern: string, name: string): string {
  const pascal = name.split(/[-_]/u).map((part) => part.charAt(0).toUpperCase() + part.slice(1)).join("");
  return pattern.replaceAll("{rule}", name).replaceAll("{Rule}", pascal);
}

/**
 * The documentation in a rule's `meta` block (its docs, option schema, type and the like): every member but
 * `defaultOptions`, whose values (no-warning-comments' `todo`, `fixme`, `xxx`) are the options the rule runs with when a
 * configuration sets none, and `messages`, the text the rule reports (`'... with an optional parameter.'`).
 */
const IMPLEMENTED_META = new Set(["defaultOptions", "messages"]);

export function ruleDocumentation(ts: TypeScript): DocumentationNode {
  const members = [ts.SyntaxKind.PropertyAssignment, ts.SyntaxKind.ShorthandPropertyAssignment, ts.SyntaxKind.SpreadAssignment,
    ts.SyntaxKind.MethodDeclaration, ts.SyntaxKind.GetAccessor, ts.SyntaxKind.SetAccessor];
  return (node) => {
    const object = node.parent;
    const owner = object?.parent;
    return members.includes(node.kind) && !IMPLEMENTED_META.has(node.name?.text ?? "")
      && object?.kind === ts.SyntaxKind.ObjectLiteralExpression && owner?.kind === ts.SyntaxKind.PropertyAssignment && owner?.name?.text === "meta";
  };
}

/**
 * How an ESLint rule's sources name what they check: its prose documentation sits in `meta`, so every word of the
 * strings left counts (data the code builds, such as the `'parameter property'` a message names, and the messages
 * themselves), and so do the keys of its lookup tables
 * (`{ Math: { pow: { [CALL]: true } } }`, conf/globals.js's `{ Array: false, ... }`).
 */
const NAMES = { everyStringWord: true, objectKeys: true } as const;

/** The file a relative module specifier names, as Node and TypeScript resolve it from `directory`. */
export function resolveModule(directory: string, specifier: string): string | undefined {
  const base = resolve(directory, specifier);
  return [base, `${base}.ts`, `${base}.js`, join(base, "index.ts"), join(base, "index.js")].find((candidate) => existsSync(candidate) && extname(candidate) !== "");
}

/** The npm package a bare module specifier names: `@scope/name` or `name`, without a subpath. */
export function packageName(specifier: string): string {
  const parts = specifier.split("/");
  return parts.slice(0, specifier.startsWith("@") ? 2 : 1).join("/");
}

/**
 * The npm packages whose modules count as upstream's code where upstream's code imports them (the manifest's
 * `helpers`): the helper libraries upstream's rules are written with, such as @eslint-community/eslint-utils.
 */
export type Helpers = ReadonlySet<string>;

/**
 * Where a module specifier leads from `file`: a relative path as Node and TypeScript resolve it, or a module of a
 * helper package as Node resolves it from the file (for a checkout file, through its link to the runtime's packages);
 * undefined for any other package.
 */
export function resolveSpecifier(file: string, specifier: string, helpers: Helpers): string | undefined {
  if (specifier.startsWith(".")) return resolveModule(dirname(file), specifier);
  if (!helpers.has(packageName(specifier))) return undefined;
  try {
    return createRequire(file).resolve(specifier);
  } catch {
    return undefined;
  }
}

/** Whether a resolved module belongs to a helper package rather than to upstream's own sources. */
export function inHelperPackage(file: string, helpers: Helpers): boolean {
  return helperPackage(file, helpers) !== undefined;
}

/** The helper package a resolved module belongs to: the innermost `node_modules/<helper>/` on its path. */
export function helperPackage(file: string, helpers: Helpers): string | undefined {
  const path = file.replaceAll("\\", "/");
  let found: { name: string; at: number } | undefined;
  for (const name of helpers) {
    const at = path.lastIndexOf(`/node_modules/${name}/`);
    if (at !== -1 && (found === undefined || at > found.at)) found = { name, at };
  }
  return found === undefined ? undefined : path.slice(0, found.at + `/node_modules/${found.name}/`.length);
}

function parse(file: string): ts.SourceFile {
  return ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.ESNext, true, file.endsWith(".ts") ? ts.ScriptKind.TS : ts.ScriptKind.JS);
}

const hasModifier = (node: ts.Node, kind: ts.SyntaxKind) => ts.canHaveModifiers(node) && (ts.getModifiers(node) ?? []).some((modifier) => modifier.kind === kind);
const hasExport = (node: ts.Node) => hasModifier(node, ts.SyntaxKind.ExportKeyword);

/** The module and exported name an import declaration binds to a local name (`import x from "m"` binds m's default). */
function importOf(source: ts.SourceFile, local: string): { specifier: string; name: string } | undefined {
  for (const statement of source.statements) {
    if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier) || statement.importClause === undefined) continue;
    const clause = statement.importClause;
    if (clause.name?.text === local) return { specifier: statement.moduleSpecifier.text, name: "default" };
    const element = clause.namedBindings !== undefined && ts.isNamedImports(clause.namedBindings)
      ? clause.namedBindings.elements.find((entry) => entry.name.text === local)
      : undefined;
    if (element !== undefined) return { specifier: statement.moduleSpecifier.text, name: (element.propertyName ?? element.name).text };
  }
  return undefined;
}

function bindingNames(name: ts.BindingName): string[] {
  return ts.isIdentifier(name) ? [name.text] : name.elements.flatMap((element) => (ts.isBindingElement(element) ? bindingNames(element.name) : []));
}

/** The specifier of `require("x")`, also wrapped as TypeScript's CommonJS output wraps it (`__importStar(require("x"))`). */
function requiredSpecifier(node: ts.Node): string | undefined {
  if (!ts.isCallExpression(node)) return undefined;
  if (ts.isIdentifier(node.expression) && node.expression.text === "require" && node.arguments.length === 1 && ts.isStringLiteral(node.arguments[0]!)) return node.arguments[0].text;
  if (ts.isIdentifier(node.expression) && /^__import(Star|Default)$/u.test(node.expression.text) && node.arguments.length === 1) return requiredSpecifier(node.arguments[0]!);
  return undefined;
}

/** `exports.name` or `module.exports.name`: the name a CommonJS module exports by assignment. */
function exportsMember(node: ts.Node): string | undefined {
  if (!ts.isPropertyAccessExpression(node)) return undefined;
  const target = node.expression;
  const isExports = (ts.isIdentifier(target) && target.text === "exports")
    || (ts.isPropertyAccessExpression(target) && ts.isIdentifier(target.expression) && target.expression.text === "module" && target.name.text === "exports");
  return isExports ? node.name.text : undefined;
}

/**
 * The file that defines a name a module exports: the module itself when it declares the export, otherwise the module
 * it re-exports the name from, followed through re-exports. ECMAScript modules re-export with `export * from` and
 * `export { name } from`; CommonJS modules (a published package's build) assign `exports.name`, re-export through
 * `Object.defineProperty(exports, "name", { get })` and `__exportStar(require(...), exports)`, and export another
 * module as a namespace with `exports.name = require(...)`, which that module then defines. Re-exports are followed
 * by relative path and into helper packages only. A default export is the module's own, unless the module exports a
 * binding it imports (`import list from "./list.json"; export default list;`), which the imported module defines; a
 * JSON module's default export is its data.
 */
export function definingModule(file: string, name: string, helpers: Helpers = new Set(), seen: Set<string> = new Set()): string | undefined {
  if (seen.has(file)) return undefined;
  seen.add(file);
  if (name === "default" && extname(file) === ".json") return file;
  const source = parse(file);
  const follow = (specifier: string, exported: string) => {
    const target = resolveSpecifier(file, specifier, helpers);
    return target === undefined ? undefined : definingModule(target, exported, helpers, seen);
  };
  for (const statement of source.statements) {
    if (name === "default" && ts.isExportAssignment(statement) && !statement.isExportEquals) {
      const imported = ts.isIdentifier(statement.expression) ? importOf(source, statement.expression.text) : undefined;
      return imported === undefined ? file : follow(imported.specifier, imported.name) ?? file;
    }
    if (name === "default" && hasExport(statement) && hasModifier(statement, ts.SyntaxKind.DefaultKeyword)) return file;
    if (ts.isExportDeclaration(statement) && statement.moduleSpecifier === undefined && statement.exportClause !== undefined && ts.isNamedExports(statement.exportClause)
      && statement.exportClause.elements.some((element) => element.name.text === name)) return file;
    if (hasExport(statement)) {
      if (ts.isVariableStatement(statement) && statement.declarationList.declarations.some((declaration) => bindingNames(declaration.name).includes(name))) return file;
      if ((ts.isFunctionDeclaration(statement) || ts.isClassDeclaration(statement) || ts.isEnumDeclaration(statement) || ts.isInterfaceDeclaration(statement)
        || ts.isTypeAliasDeclaration(statement)) && statement.name?.text === name) return file;
    }
  }
  // CommonJS: `exports.name = value` defines the name here (TypeScript's `exports.a = exports.b = void 0` only declares
  // it); `exports.name = require(...)` exports that module as a namespace.
  let assigned: string | null | undefined;
  const findAssignment = (node: ts.Node): void => {
    if (assigned !== undefined) return;
    if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken && exportsMember(node.left) === name) {
      let value: ts.Expression = node.right;
      while (ts.isBinaryExpression(value) && value.operatorToken.kind === ts.SyntaxKind.EqualsToken && exportsMember(value.left) !== undefined) value = value.right;
      if (!ts.isVoidExpression(value)) {
        const namespace = requiredSpecifier(value);
        assigned = namespace === undefined ? file : resolveSpecifier(file, namespace, helpers) ?? null;
        return;
      }
    }
    ts.forEachChild(node, findAssignment);
  };
  findAssignment(source);
  if (assigned !== undefined) return assigned ?? undefined;
  // What CommonJS bindings hold a required module: `var util_1 = require("./util")`.
  const required = new Map<string, string>();
  for (const statement of source.statements) {
    if (!ts.isVariableStatement(statement)) continue;
    for (const declaration of statement.declarationList.declarations) {
      const specifier = declaration.initializer === undefined ? undefined : requiredSpecifier(declaration.initializer);
      if (ts.isIdentifier(declaration.name) && specifier !== undefined) required.set(declaration.name.text, specifier);
    }
  }
  for (const statement of source.statements) {
    if (ts.isExportDeclaration(statement) && statement.moduleSpecifier !== undefined && ts.isStringLiteral(statement.moduleSpecifier)) {
      const named = statement.exportClause !== undefined && ts.isNamedExports(statement.exportClause) ? statement.exportClause.elements : undefined;
      const element = named?.find((entry) => entry.name.text === name);
      if (named !== undefined && element === undefined) continue;
      const found = follow(statement.moduleSpecifier.text, element?.propertyName?.text ?? name);
      if (found !== undefined) return found;
    }
    if (!ts.isExpressionStatement(statement) || !ts.isCallExpression(statement.expression)) continue;
    const call = statement.expression;
    const callee = call.expression;
    // __exportStar(require("./x"), exports)
    if (ts.isIdentifier(callee) && callee.text === "__exportStar" && call.arguments.length >= 1) {
      const specifier = requiredSpecifier(call.arguments[0]!);
      const found = specifier === undefined ? undefined : follow(specifier, name);
      if (found !== undefined) return found;
    }
    // Object.defineProperty(exports, "name", { get: function () { return x_1.original; } })
    if (ts.isPropertyAccessExpression(callee) && callee.getText(source) === "Object.defineProperty" && call.arguments.length === 3
      && ts.isIdentifier(call.arguments[0]!) && call.arguments[0].text === "exports"
      && ts.isStringLiteral(call.arguments[1]!) && call.arguments[1].text === name && ts.isObjectLiteralExpression(call.arguments[2]!)) {
      let source_: { binding: string; exported: string } | undefined;
      const findReturn = (node: ts.Node): void => {
        if (source_ !== undefined) return;
        if (ts.isReturnStatement(node) && node.expression !== undefined && ts.isPropertyAccessExpression(node.expression) && ts.isIdentifier(node.expression.expression)) {
          source_ = { binding: node.expression.expression.text, exported: node.expression.name.text };
          return;
        }
        ts.forEachChild(node, findReturn);
      };
      findReturn(call.arguments[2]);
      const specifier = source_ === undefined ? undefined : required.get(source_.binding);
      const found = specifier === undefined ? undefined : follow(specifier, source_!.exported);
      if (found !== undefined) return found;
    }
  }
  // A CommonJS module's default export is what it assigns to module.exports: the module's own.
  return name === "default" ? file : undefined;
}

/** The modules a rule delegates to: upstream's own, followed transitively, and helper packages' defining modules. */
export interface DelegatedModules {
  own: string[];
  helpers: string[];
}

/**
 * The upstream modules a rule's implementation delegates to (or any other entry module's, such as the parser an
 * approved configuration sets), followed through what they import in turn (ESLint's `astUtils.ECMASCRIPT_GLOBALS` is
 * conf/globals.js's table): each module imported or required by relative path, other rules aside (modules directly in
 * `excluded`), and for a module that only re-exports what it imports by name (a barrel such as typescript-eslint's
 * `../util`), the modules that define the names imported or read from it. A name imported from a helper package counts
 * through the module that defines it. That module's imports of its own package are the package's internals and are not
 * followed. Of what it imports from another helper package only data counts, a JSON module the helper reads as a whole
 * (eslint-plugin-unicorn's prefer-node-protocol calls is-builtin-module, which checks names against builtin-modules'
 * list). Another package's code reached that way is not the rule's: @typescript-eslint/utils re-exports token predicates
 * from @eslint-community/eslint-utils, a single-file bundle whose strings are the whole library's.
 */
export function importedModules(implementation: string, helpers: Helpers = new Set(), excluded: string | null = dirname(implementation)): DelegatedModules {
  const own = new Set<string>();
  const helperModules = new Set<string>();
  const pending = [implementation];
  for (let file = pending.shift(); file !== undefined; file = pending.shift()) {
    const fromHelper = helperPackage(file, helpers);
    for (const target of directImports(file, fromHelper === undefined ? excluded : null, helpers)) {
      const helper = helperPackage(target, helpers);
      if (helper !== undefined) {
        if (helper === fromHelper || helperModules.has(target)) continue;
        if (fromHelper !== undefined && extname(target) !== ".json") continue;
        helperModules.add(target);
        pending.push(target);
        continue;
      }
      if (fromHelper !== undefined || target === implementation || own.has(target)) continue;
      own.add(target);
      pending.push(target);
    }
  }
  return { own: [...own].sort(), helpers: [...helperModules].sort() };
}

/** The names read from a binding as `binding.name` (and TypeScript's `(0, binding.name)(...)`), in source order. */
function namesRead(source: ts.SourceFile, binding: string): string[] {
  const read = new Set<string>();
  const collect = (node: ts.Node): void => {
    if (ts.isPropertyAccessExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === binding) read.add(node.name.text);
    ts.forEachChild(node, collect);
  };
  collect(source);
  return [...read].sort();
}

/**
 * The modules one file imports, as `importedModules` reads them: by relative path outside `excluded`, and from helper
 * packages. A relative `require` delegates to the whole module; from a helper package, a `require` delegates to the
 * names destructured from it or read from the binding that holds it.
 */
function directImports(file: string, excluded: string | null, helpers: Helpers): string[] {
  const source = parse(file);
  const modules = new Set<string>();
  const use = (specifier: string, names: string[] | null) => {
    const target = resolveSpecifier(file, specifier, helpers);
    if (target === undefined || (excluded !== null && dirname(target) === excluded)) return;
    const defining = names === null ? [] : names.map((name) => definingModule(target, name, helpers));
    // A module that defines a name it is asked for, or is asked for nothing by name, is delegated to as a whole.
    if (names === null || defining.includes(target)) modules.add(target);
    for (const defined of defining) if (defined !== undefined) modules.add(defined);
  };
  const requiredNames = (call: ts.CallExpression): string[] | null => {
    let node: ts.Node = call;
    while (ts.isCallExpression(node.parent) && requiredSpecifier(node.parent) !== undefined) node = node.parent;
    const parent = node.parent;
    if (ts.isPropertyAccessExpression(parent) && parent.expression === node) return [parent.name.text];
    if (!ts.isVariableDeclaration(parent) || parent.initializer !== node) return null;
    if (ts.isObjectBindingPattern(parent.name)) {
      return parent.name.elements.map((element) => (element.propertyName !== undefined && ts.isIdentifier(element.propertyName) ? element.propertyName.text : ts.isIdentifier(element.name) ? element.name.text : "")).filter(Boolean);
    }
    if (!ts.isIdentifier(parent.name)) return null;
    const read = namesRead(source, parent.name.text);
    return read.length === 0 ? null : read;
  };
  const visit = (node: ts.Node): void => {
    if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier) && !node.importClause?.isTypeOnly) {
      const bindings = node.importClause?.namedBindings;
      if (bindings !== undefined && ts.isNamedImports(bindings)) {
        use(node.moduleSpecifier.text, bindings.elements.filter((element) => !element.isTypeOnly).map((element) => (element.propertyName ?? element.name).text));
      } else if (bindings !== undefined && ts.isNamespaceImport(bindings)) {
        // The names a rule reads from a namespace import are the helpers it calls.
        use(node.moduleSpecifier.text, namesRead(source, bindings.name.text));
      } else if (node.importClause?.name !== undefined) {
        // A default export is the module's own; a helper package's is defined by the module its entry exports it from.
        use(node.moduleSpecifier.text, node.moduleSpecifier.text.startsWith(".") ? null : ["default"]);
      }
    } else if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "require"
      && node.arguments.length === 1 && ts.isStringLiteral(node.arguments[0]!)) {
      const specifier = node.arguments[0].text;
      use(specifier, specifier.startsWith(".") ? null : requiredNames(node));
    }
    ts.forEachChild(node, visit);
  };
  visit(source);
  return [...modules].sort();
}

/** The string literals of a source that are single words, in source order: `"off"`, `"warn"`, not `"use strict"`. */
function stringWords(node: ts.Node, found: string[] = []): string[] {
  if (ts.isStringLiteral(node) && /^[A-Za-z_]\w*$/u.test(node.text)) found.push(node.text);
  ts.forEachChild(node, (child) => void stringWords(child, found));
  return found;
}

/**
 * The values ESLint's directive comments take, from the pinned eslint's sources: a global's setting in
 * `/* global a:off *\/` (the case labels of source-code.js's normalizeConfigGlobal) and a rule's severity in
 * `/* eslint semi: "warn" *\/` (the strings of shared/severity.js).
 */
export function directiveValuesFrom(sourceCode: string, severity: string): string[] {
  const settings: string[] = [];
  const visit = (node: ts.Node): void => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === "normalizeConfigGlobal") {
      const cases = (child: ts.Node): void => {
        if (ts.isCaseClause(child) && ts.isStringLiteral(child.expression)) settings.push(child.expression.text);
        ts.forEachChild(child, cases);
      };
      cases(node);
    } else ts.forEachChild(node, visit);
  };
  visit(ts.createSourceFile("source-code.js", sourceCode, ts.ScriptTarget.ESNext, true, ts.ScriptKind.JS));
  const severities = stringWords(ts.createSourceFile("severity.js", severity, ts.ScriptTarget.ESNext, true, ts.ScriptKind.JS));
  if (settings.length === 0 || severities.length === 0) throw new Error("cannot find the global settings (normalizeConfigGlobal) or the severities (shared/severity.js) in the pinned eslint");
  return [...new Set([...settings, ...severities])].sort();
}

function directiveValues(eslintDirectory: string): string[] {
  const read = (path: string) => readFileSync(join(eslintDirectory, path), "utf8");
  return directiveValuesFrom(read("lib/languages/js/source-code/source-code.js"), read("lib/shared/severity.js"));
}

/**
 * The version a source file declares for a package: the string an object literal in it maps the package's name to
 * (facebook/react's ReactVersions.js, `'eslint-plugin-react-hooks': '7.1.1'`); undefined when none does.
 */
export function declaredVersion(source: string, packageName: string): string | undefined {
  let found: string | undefined;
  const visit = (node: ts.Node): void => {
    if (found !== undefined) return;
    if (ts.isPropertyAssignment(node) && ts.isObjectLiteralExpression(node.parent) && (ts.isStringLiteral(node.name) || ts.isIdentifier(node.name))
      && node.name.text === packageName && ts.isStringLiteral(node.initializer)) {
      found = node.initializer.text;
      return;
    }
    ts.forEachChild(node, visit);
  };
  visit(ts.createSourceFile("versions.js", source, ts.ScriptTarget.ESNext, true, ts.ScriptKind.JS));
  return found;
}

/** Test fixture files are read as source code when they have one of these extensions, and as plain words otherwise. */
const SOURCE_EXTENSIONS = new Set([".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".cts", ".tsx"]);

/** The literal directories a glob starts with, so only those are walked. */
function globRoot(glob: string): string {
  const parts = glob.split("/");
  const wild = parts.findIndex((part) => /[*?[{]/u.test(part));
  return parts.slice(0, wild === -1 ? parts.length - 1 : wild).join("/");
}

/** The files of the checkout that match the manifest's `test_fixtures` globs, in path order. */
export function testFixtureFiles(checkout: string, globs: string[]): string[] {
  const matches = (path: string) => globs.some((glob) => matchesGlob(relative(checkout, path).replaceAll("\\", "/"), glob));
  return [...new Set(globs.flatMap((glob) => filesUnder(join(checkout, globRoot(glob)), matches)))].sort();
}

/**
 * A file at the root of upstream's checkout by name, in any letter case (eslint-plugin-unicorn keeps its license as
 * `license`); the exact name first. Undefined when there is none.
 */
export function rootFile(directory: string, name: string): string | undefined {
  const entries = readdirSync(directory);
  const found = entries.includes(name) ? name : entries.filter((entry) => entry.toUpperCase() === name.toUpperCase()).sort()[0];
  return found === undefined ? undefined : join(directory, found);
}

/** The sparse checkout's link to the runtime's packages, so upstream's test files resolve what they import from it. */
async function linkRuntime(checkout: string, runtime: string): Promise<void> {
  const link = join(checkout, "node_modules");
  const target = join(runtime, "node_modules");
  // Packs validated side by side share a checkout: relink only when the link resolves elsewhere, and swap it in with one
  // rename so a concurrent run never finds it missing.
  if ((await realpath(link).catch(() => null)) !== (await realpath(target))) {
    const staged = `${link}.${process.pid}`;
    await symlink(target, staged, "dir");
    await rename(staged, link);
  }
  // Ignored, so the checkout stays unmodified for checkoutUpstream.
  const exclude = join(checkout, ".git", "info", "exclude");
  const text = existsSync(exclude) ? await readFile(exclude, "utf8") : "";
  if (!text.split("\n").includes("/node_modules")) {
    await mkdir(dirname(exclude), { recursive: true });
    await appendFile(exclude, `${text === "" || text.endsWith("\n") ? "" : "\n"}/node_modules\n`);
  }
}

export async function createEslintAdapter(pack: PackManifest, names: string[], cacheRoot: string, runner: CommandRunner): Promise<PackAdapter> {
  const setup = pack.eslint;
  if (setup === undefined) throw new Error(`${pack.id}: the eslint adapter needs the manifest's eslint block`);
  const repositoryName = pack.upstream.repository.slice(pack.upstream.repository.lastIndexOf("/") + 1);
  const upstream = await checkoutUpstream(cacheRoot, repositoryName, pack.upstream, setup.release.tag, setup.release.path, setup.checkout, runner);
  const runtime = await npmRuntime(pack.upstream.runtime, cacheRoot, runner);
  await linkRuntime(upstream.directory, runtime);
  // Independent parsers for a second opinion on source the engine cannot parse, from runtimes/parsers/package.json.
  const parsers = createRequire(join(await npmRuntime("parsers", cacheRoot, runner), "package.json"));
  const environmentRuntime = setup.environment === undefined ? null : await npmRuntime(setup.environment.runtime, cacheRoot, runner);
  const require = createRequire(join(runtime, "package.json"));
  const runtimeTs = require("typescript") as TypeScript;
  const independent = {
    fixtures: runtimeTs,
    current: parsers("typescript") as TypeScript,
    babel: parsers("@babel/parser") as BabelParser,
    babelVersion: (parsers("@babel/parser/package.json") as { version: string }).version,
  };

  const bare = (name: string) => bareRuleName(setup.plugin, name);
  const ruleId = (name: string) => (setup.plugin === null ? name : `${setup.plugin.namespace}/${name}`);
  const ruleNames = [...new Set(names.map(bare))].sort();
  // Test files written in TypeScript load through the runtime's own tsx, when it installs one.
  const tsx = join(runtime, "node_modules", "tsx", "dist", "loader.mjs");
  const node = [process.execPath, ...(existsSync(tsx) ? ["--import", tsx] : []), DRIVER];
  const eslintPackage = setup.eslint_package ?? "eslint";
  const context = {
    runtime, checkout: upstream.directory, cache: cacheRoot, plugin: setup.plugin, configs: pack.upstream.recommended_configs,
    release: { path: setup.release.path, installed: setup.release.installed, defaultExport: setup.release.default_export === true },
    fixtureParsers: setup.fixture_parsers ?? {}, namedParsers: setup.parsers ?? {}, eslintPackage,
    environment: environmentRuntime === null ? null : { runtime: environmentRuntime, package: setup.environment!.package },
  };
  // Upstream's tests run where its own test runner runs them, when what they load reads the working directory.
  const cwd = setup.test_cwd === undefined ? runtime : join(upstream.directory, setup.test_cwd);
  const ask = async <T>(request: object): Promise<T> => {
    await mkdir(resolve(cacheRoot, "tmp"), { recursive: true });
    const directory = await mkdtemp(resolve(cacheRoot, "tmp", "eslint-"));
    try {
      const out = join(directory, "result.json");
      await runChecked(runner, node, { input: JSON.stringify({ ...context, ...request, out }), cwd });
      return JSON.parse(await readFile(out, "utf8")) as T;
    } finally {
      await rm(directory, { recursive: true, force: true });
    }
  };
  const facts = await ask<Facts>({ op: "facts", names: ruleNames });
  const tests = await ask<Tests>({ op: "tests", tests: setup.tests, names: ruleNames, caseTesters: setup.case_testers ?? [] });

  const upstreamPath = (absolute: string) => relative(upstream.directory, absolute).replaceAll("\\", "/");
  const implementationFile = (name: string) => {
    const path = join(upstream.directory, implementationPath(setup.implementation, name));
    return existsSync(path) ? path : undefined;
  };
  const packageRoot = dirname(setup.release.path);
  const upstreamPackage = JSON.parse(readFileSync(join(upstream.directory, packageRoot, "package.json"), "utf8")) as { license?: string; version?: string };
  const packageJson = `${packageRoot === "." ? "" : `${packageRoot}/`}package.json`;
  const packageId = setup.plugin?.package ?? "eslint";
  const installedVersion = setup.plugin === null ? facts.eslint : facts.plugin;
  const installed = join(runtime, "node_modules", setup.release.installed);
  // The version the source at the pinned commit declares: its package.json's, or the one the manifest's version file
  // maps the package to, where upstream stamps the version when it builds.
  const versionFile = setup.release.version_file;
  const sourceVersion = versionFile === undefined ? upstreamPackage.version : declaredVersion(readFileSync(join(upstream.directory, versionFile), "utf8"), packageId);

  const packChecks: Check[] = [
    ...upstream.checks,
    {
      id: "runtime-version",
      ok: installedVersion === pack.upstream.version && sourceVersion === pack.upstream.version,
      detail: `installed ${packageId} is ${installedVersion}; ${versionFile === undefined ? "" : `${versionFile} at the pinned commit maps ${packageId} to ${sourceVersion}, and `}${packageJson} at the pinned commit says ${upstreamPackage.version}; the pack pins ${pack.upstream.version}`,
    },
  ];
  if (setup.environment !== undefined) {
    const environment = setup.environment;
    const installedEnvironment = (JSON.parse(readFileSync(join(environmentRuntime!, "node_modules", environment.package, "package.json"), "utf8")) as { version?: string }).version;
    packChecks.push({
      id: "environment-version",
      ok: installedEnvironment === environment.version,
      detail: `the environment ${environment.package} is installed at ${installedEnvironment ?? "no version"}; the pack pins ${environment.version}, built from ${environment.source.repository} ${environment.source.path} at ${environment.source.commit}`,
    });
  }
  if (setup.release.verbatim) {
    packChecks.push({
      id: "runtime-matches-checkout",
      ok: treeDigest(installed) === treeDigest(join(upstream.directory, setup.release.path)),
      detail: `the installed ${setup.release.installed} and ${setup.release.path}/ at the pinned commit have the same files and content`,
    });
  }
  if (setup.plugin === null) {
    // eslint:recommended is @eslint/js's, which ESLint versions apart from eslint itself.
    const jsPackage = JSON.parse(readFileSync(join(upstream.directory, ESLINT_JS, "package.json"), "utf8")) as { version?: string };
    const sameSource = treeDigest(join(runtime, "node_modules", "@eslint", "js", "src")) === treeDigest(join(upstream.directory, ESLINT_JS, "src"));
    packChecks.push({
      id: "recommended-matches-checkout",
      ok: facts.configs.version === jsPackage.version && sameSource,
      detail: `the installed @eslint/js is ${facts.configs.version} and ${ESLINT_JS}/package.json at the pinned commit says ${jsPackage.version}; its src/ and ${ESLINT_JS}/src/ have ${sameSource ? "the same" : "different"} files and content`,
    });
  }
  packChecks.push({
    id: "declared-license",
    ok: upstreamPackage.license === pack.upstream.license,
    detail: `${packageJson} at the pinned commit declares ${upstreamPackage.license}; the pack records ${pack.upstream.license}`,
  });
  const loadErrors = tests.files.filter((file) => file.error !== null);
  packChecks.push({
    id: "upstream-tests-load",
    ok: loadErrors.length === 0 && tests.files.length > 0,
    detail: loadErrors.length === 0
      ? `${tests.files.length} upstream test file(s) load with upstream's RuleTester`
      : `${loadErrors.length} upstream test file(s) do not load, e.g. ${loadErrors[0]!.file}: ${loadErrors[0]!.error}`,
  });

  const isMeta = ruleDocumentation(runtimeTs);
  const helpers: Helpers = new Set(setup.helpers);
  // Every rule reads the scopes the approved configurations' parser builds: the helper modules that parser calls
  // (typescript-eslint's parser calls @typescript-eslint/scope-manager's analyze, whose default JSX pragma is React).
  const parserHelpers = [...new Set(facts.parsers.flatMap((parser) => importedModules(parser, helpers, null).helpers))];
  const delegated = new Map<string, string[]>();
  const sharedModules = (name: string): string[] => {
    const known = delegated.get(name);
    if (known !== undefined) return known;
    const implementation = implementationFile(name);
    const own = implementation === undefined ? { own: [], helpers: [] } : importedModules(implementation, helpers);
    // A rule that extends an ESLint rule runs that rule's implementation in the installed eslint, and what it imports.
    const base = facts.rules[name]?.extendsBaseRule;
    const baseRule = base === null || base === undefined ? undefined : join(dirname(require.resolve(`${eslintPackage}/package.json`)), "lib", "rules", `${base}.js`);
    const inherited = baseRule !== undefined && existsSync(baseRule) ? importedModules(baseRule, helpers) : undefined;
    const modules = [...new Set([...own.own, ...own.helpers, ...(inherited === undefined ? [] : [baseRule!, ...inherited.own, ...inherited.helpers]), ...parserHelpers])].sort();
    delegated.set(name, modules);
    return modules;
  };

  // not-overfit's test words: every case of every upstream test file (its code, its expected outputs, its test
  // configuration and what it expects reported), and the fixture sources upstream's tests load.
  const fixtureFiles = testFixtureFiles(upstream.directory, setup.test_fixtures ?? []);
  const fixtureWords = new Set<string>();
  for (const { code, jsx } of tests.texts) for (const word of sourceWords(runtimeTs, code, jsx)) fixtureWords.add(word);
  for (const configuration of tests.configurations) for (const word of words(configuration)) fixtureWords.add(word);
  const fixtureSources: string[] = [];
  for (const file of fixtureFiles) {
    const text = readFileSync(file, "utf8");
    if (SOURCE_EXTENSIONS.has(extname(file))) {
      fixtureSources.push(text);
      for (const word of sourceWords(runtimeTs, text, /x$/u.test(extname(file)))) fixtureWords.add(word);
    } else {
      for (const word of words(text)) fixtureWords.add(word);
    }
  }
  const fixtureTexts = [...new Set([...tests.texts.map(({ code }) => code), ...fixtureSources])];
  // Each text's origin, in the same order: a test's code inline, a fixture source by its path in the checkout.
  const sourceOf = new Map<string, FixtureTextSource>();
  for (const { code } of tests.texts) if (!sourceOf.has(code)) sourceOf.set(code, { inline: code });
  fixtureFiles.filter((file) => SOURCE_EXTENSIONS.has(extname(file))).forEach((file, index) => {
    const text = fixtureSources[index]!;
    if (!sourceOf.has(text)) sourceOf.set(text, { path: relative(upstream.directory, file).split(sep).join("/"), sha256: createHash("sha256").update(text).digest("hex") });
  });
  const fixtureTextSources = fixtureTexts.map((text) => sourceOf.get(text)!);

  /** Whether upstream reports end positions for the rule: every finding on its own tests carries both ends. */
  const endsReported = new Map(ruleNames.map((name) => [name, (tests.cases[name] ?? []).every((testCase) => testCase.findings.every(([, , endLine, endColumn]) => endLine !== null && endColumn !== null))]));
  const extensions = new Set(facts.extensions);
  // The license file the manifest names (vercel/next.js's license.md), or LICENSE at upstream's root in any letter case.
  const licenseFile = setup.license === undefined ? rootFile(upstream.directory, "LICENSE") ?? join(upstream.directory, "LICENSE") : join(upstream.directory, setup.license);

  return {
    identity: {
      repository: pack.upstream.repository,
      commit: pack.upstream.commit,
      version: pack.upstream.version,
      // The runtime's pinned packages, not Node: Node is whichever the recording machine runs (CI's floats within a
      // major), and a recording names nothing of the machine, so the nightly check compares like with like.
      runtime: [`eslint ${facts.eslint}`, ...(setup.plugin === null ? [] : [`${setup.plugin.package} ${facts.plugin}`]), `${facts.configs.package} ${facts.configs.version}`,
        `typescript ${runtimeTs.version}`].filter((part, index, parts) => parts.indexOf(part) === index).join(", "),
      ...(setup.environment === undefined ? {} : {
        environment: `${setup.environment.package} ${setup.environment.version} (${setup.environment.source.repository} ${setup.environment.source.path} at ${setup.environment.source.commit}): its file globs, parsers and globals, without its plugins, rules or settings; a test case's own parser and globals give way to them`,
      }),
    },
    packChecks,
    licenseText: readFileSync(licenseFile, "utf8"),

    rule(name: string): UpstreamRule | undefined {
      const id = bare(name);
      const fact = facts.rules[id];
      const implementation = implementationFile(id);
      if ((fact === undefined || fact === null) && implementation === undefined) return undefined;
      return {
        name: ruleId(id),
        implementation: implementation === undefined ? [] : [upstreamPath(implementation)],
        tests: tests.testFiles[id] ?? [],
        deprecation: fact?.deprecated ?? null,
        recommended: fact?.recommended ?? null,
        notOnRealCode: fact?.requiresTypeChecking === true
          ? "upstream runs it only with type information from a TypeScript project, and corpus files are linted one at a time without one"
          : null,
      };
    },

    implementationNames(name: string): Set<string> {
      const implementation = implementationFile(bare(name));
      return implementation === undefined ? new Set() : literalNames(runtimeTs, [implementation], isMeta, NAMES);
    },
    sharedImplementationNames: (name: string) => literalNames(runtimeTs, sharedModules(bare(name)), isMeta, NAMES),
    implementationRegexes(name: string): RegExp[] {
      const implementation = implementationFile(bare(name));
      return literalRegexes(runtimeTs, [...(implementation === undefined ? [] : [implementation]), ...sharedModules(bare(name))], isMeta);
    },
    fixtureWords: () => fixtureWords,
    fixtureTexts: () => fixtureTexts,
    fixtureTextSources: () => fixtureTextSources,

    keywords: keywords(runtimeTs),
    // TypeScript's JSDoc tags and pragmas, ESLint's directive comments (`eslint-disable-next-line`, `global`) and the
    // values its directives take (`/* global a:off */`, `/* eslint semi: "warn" */`).
    commentVocabulary: new Set([
      ...commentVocabulary(runtimeTs, readFileSync(require.resolve("typescript"), "utf8")),
      ...facts.directives.flatMap((directive) => directive.split("-")),
      ...directiveValues(dirname(require.resolve(`${eslintPackage}/package.json`))),
    ]),
    // One sample per file type the approved configurations make upstream lint.
    inScopeSamples: facts.extensions.map((extension) => `src/example${extension}`),

    cases(name: string): UpstreamCase[] {
      const id = bare(name);
      const ends = endsReported.get(id) ?? true;
      return (tests.cases[id] ?? []).map((testCase) => ({
        fixture: testCase.fixture,
        variant: testCase.variant,
        path: testCase.path,
        ...(testCase.parsedAs === undefined ? {} : { parsedAs: testCase.parsedAs }),
        source: testCase.code,
        script: testCase.script === true,
        skipped: testCase.skipped,
        expected: testCase.findings.map((position) => messageSpan(position, ends)),
        markup: testCase.markup === null ? null : testCase.markup.map((position) => messageSpan(position, ends)),
      }));
    },

    toSpan(name: string, source: string, startByte: number, endByte: number): Span {
      return eslintSpan(source, startByte, endByte, eslintSpanParts(endsReported.get(bare(name)) ?? true));
    },

    // The file types upstream's approved configurations lint; a declaration file is a TypeScript file.
    inCorpus: (path: string) => extensions.has(extname(path).toLowerCase()),

    async lintCorpus(files, ruleNamesToLint): Promise<UpstreamFile[]> {
      if (files.length === 0) return [];
      const linted = await ask<Linted[]>({ op: "lint", names: [...new Set(ruleNamesToLint.map(bare))].sort(), files: files.map((file) => ({ path: `${file.task}/${file.path}`, source: file.source })) });
      return files.map((file, index) => {
        const result = linted[index]!;
        if (result.excluded !== null) return { file, excluded: result.excluded, findings: new Map() };
        return {
          file,
          excluded: null,
          findings: new Map(ruleNamesToLint.map((name) => [name, (result.findings[bare(name)] ?? []).map((position) => messageSpan(position, endsReported.get(bare(name)) ?? true))])),
        };
      });
    },

    async parserVerdicts(files) {
      return files.map(({ path, source, script }) => parserVerdicts(independent, path, source, script === true));
    },
  };
}
