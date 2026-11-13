/**
 * ESLint's flat config (`eslint.config.{js,mjs,cjs,ts,mts,cts}`) read as data: the file is parsed, never run or
 * imported. A file is read only when it is data throughout: imports, top-level `const`s, the export, and type-only
 * declarations, built from literals, arrays, objects without methods, spreads, reads of members, and calls that are given
 * no local `const` (a config helper excepted); anything else (a function, a statement that changes something, a
 * namespace, a loop) is code, and the file is not read. Nothing in such a file changes a `const` once set, so the rule
 * entries of the exported config array are read through same-file `const` bindings, array and object spreads,
 * `defineConfig(...)` (from `eslint/config` or `@eslint/config-helpers`), typescript-eslint's `config(...)` and
 * `extends`. Rerail takes those helpers, and the modules the file imports, to do what their documentation says; it does
 * not read them. A shared config it recognises by the module the file imports and the member path it reads
 * (`js.configs.recommended`, `...tseslint.configs.strict`, `...react.configs.flat.recommended` in an object,
 * `...js.configs.recommended.rules` in `rules`: `eslint-presets.ts`) contributes its rule entries in place, marked with
 * the preset. Anything else the file takes from them (an unrecognised shared config, an imported value, a call's
 * result) is not evaluated: a rule entry whose value it decides is reported as unresolved, and a config element it is is
 * named in a note and in `unread`.
 */
import { createRequire } from "node:module";
import type * as BabelParser from "@babel/parser";
import type * as t from "@babel/types";
import { flatPreset } from "./eslint-presets.js";
import type { EslintPresetEntry } from "./generated/eslint-presets.generated.js";
import { LintConfigFormatError } from "./formats/lint-config-formats.js";

/** The parser, loaded on the first flat config: every `rerail` command imports this module, few parse one. */
let babel: typeof BabelParser | undefined;
const loadParser = (): typeof BabelParser => (babel ??= createRequire(import.meta.url)("@babel/parser") as typeof BabelParser);

/** Where a flat config's rule entry applies. */
export type FlatConfigScope =
  /** Every file: the config object has no `files`, `ignores` or `basePath`. */
  | { readonly kind: "all" }
  /** Exactly the files under these folders (relative to the repository root), from `files: ["<folder>/**"]`. */
  | { readonly kind: "folders"; readonly folders: readonly string[]; readonly files: string }
  /** Some files Rerail does not map onto folders; `files` is how the config names them. */
  | { readonly kind: "some"; readonly files: string };

/** A rule entry of a config object, in config order. */
export interface FlatConfigEntry {
  readonly key: string;
  readonly scope: FlatConfigScope;
  /** The entry's value as JSON, or why it cannot be read without running the config. */
  readonly value: { readonly ok: true; readonly value: unknown } | { readonly ok: false; readonly why: string };
  /** What comes after the entry that Rerail does not read and that may replace it (a shared config, a spread). */
  readonly shadowedBy?: string;
  /** The recognised shared config the entry comes from, as the file names it (`js.configs.recommended`). */
  readonly preset?: string;
}

type Entry = { -readonly [K in keyof FlatConfigEntry]: FlatConfigEntry[K] };

/** A recognised shared config a flat config names: how the file names it, and its entries. */
interface Preset {
  readonly text: string;
  readonly entries: readonly EslintPresetEntry[];
}

/** A config object property a recognised shared config object spread into it gives: the preset, and the value as JSON. */
class PresetValue {
  constructor(readonly preset: string, readonly json: unknown) {}
}

/** A config object's property: an expression of the file, or what a spread shared config gives. */
type Property = t.Expression | PresetValue;

export interface FlatConfigRead {
  readonly entries: readonly FlatConfigEntry[];
  /** What the config holds that is not read and may set any rule after it (an unrecognised shared config, a spread), as the file names it. */
  readonly unread: readonly string[];
  /** What the config holds that is not read, for the person. */
  readonly notes: readonly string[];
}

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
type Unresolved = { readonly ok: false; readonly why: string };
type Resolved = { readonly ok: true; readonly value: Json } | Unresolved;

const MAX_SOURCE_TEXT = 48;
const DEFINE_CONFIG_MODULES = new Set(["eslint/config", "@eslint/config-helpers"]);
const TYPESCRIPT_ESLINT_MODULES = new Set(["typescript-eslint"]);
/** How deep a value may nest or a binding chain may go before it is not read. */
const MAX_DEPTH = 64;
/**
 * How much work reading one file may take: the reader steps into a value (`#step`) and builds JSON values (`#built`).
 * A const named twice (`const b = [a, a]`), a choice with it on both sides, or an array spread twice doubles both at
 * each level, so 40 such levels in a small file would take 2^40 steps. A config written by hand takes far less: 1,000
 * rules set to `["error", { ... }]` with ten options each take 13,000 steps and build 12,000 values.
 */
const MAX_STEPS = 100_000;
const MAX_VALUES = 100_000;
/**
 * The only globals a data file may name outside its trusted calls (`#trustedCall`): any other name the file neither
 * declares nor imports (`require`, `module`, `process`, `globalThis`, `eval`, `arguments`, ...) can reach the
 * exported config or run code.
 */
const DATA_GLOBALS = new Set(["undefined", "NaN", "Infinity", "__dirname", "__filename"]);
/** Node modules that run code, or reach the module system or the process, and so can change anything when called. */
const CODE_MODULES = new Set(["vm", "module", "inspector", "repl", "process", "worker_threads", "child_process", "v8", "wasi"]);
/** Keys through which a value reaches `Function`, a prototype, or its own accessors. */
const REFLECTIVE_KEYS = new Set([
  "constructor", "prototype", "__proto__", "__defineGetter__", "__defineSetter__", "__lookupGetter__", "__lookupSetter__",
]);
/**
 * Keys JavaScript calls by itself: when it turns an object into a primitive or JSON, and `then` when it awaits a value
 * (ESLint awaits the exported config).
 */
const COERCION_KEYS = new Set(["toString", "valueOf", "toJSON", "toLocaleString", "then"]);

/** Whether `module` is named by a path (which may be the config itself or a file of the repository), not a package. */
const isPathModule = (module: string): boolean => /^(?:\.|\/|[A-Za-z]:|file:)/u.test(module);

/** TypeScript's wrappers around an expression (`x as T`, `x satisfies T`, `x!`, `<T>x`): erased before the file runs. */
type TypeWrapper = t.TSAsExpression | t.TSSatisfiesExpression | t.TSNonNullExpression | t.TSTypeAssertion;
const TYPE_WRAPPERS = new Set(["TSAsExpression", "TSSatisfiesExpression", "TSNonNullExpression", "TSTypeAssertion"]);
const isTypeWrapper = (node: t.Node): node is TypeWrapper => TYPE_WRAPPERS.has(node.type);

/** The name of a member or object key, or undefined when it is computed from anything but a literal. */
function keyName(key: t.Node, computed: boolean): string | undefined {
  if (!computed && key.type === "Identifier") return key.name;
  if (key.type === "StringLiteral") return key.value;
  if (key.type === "NumericLiteral") return String(key.value);
  return undefined;
}

/** `<folder>/**` or `<folder>/**\/*`: every file under one folder. A `./` start is allowed. */
export function folderOfGlob(pattern: string): string | undefined {
  const match = /^(?:\.\/)?((?:[A-Za-z0-9_@.-]+\/)*[A-Za-z0-9_@.-]+)\/\*\*(?:\/\*)?$/u.exec(pattern);
  if (match === null) return undefined;
  const folder = match[1]!;
  return folder.split("/").some(part => part === "." || part === "..") ? undefined : folder;
}

/**
 * The scope of `files` patterns (JSON, as written `text`) for an element every file gets otherwise: whole folders where
 * every pattern names one, or some files.
 */
export function filesScope(patterns: unknown, text: string): FlatConfigScope {
  const folders = Array.isArray(patterns) ? patterns.map(pattern => (typeof pattern === "string" ? folderOfGlob(pattern) : undefined)) : [];
  return folders.length > 0 && folders.every(folder => folder !== undefined)
    ? { kind: "folders", folders: folders as string[], files: text }
    : { kind: "some", files: text };
}

/** Where a shared config's entry applies in an element for `scope`: its own `files` and `ignores` narrow it. */
export function presetScope(entry: EslintPresetEntry, scope: FlatConfigScope): FlatConfigScope {
  if (entry.files === undefined && entry.ignores === undefined) return scope;
  const text = entry.files === undefined ? "some files" : JSON.stringify(entry.files);
  if (scope.kind !== "all") return { kind: "some", files: `${scope.files} and ${text}` };
  return entry.ignores === undefined ? filesScope(entry.files, text) : { kind: "some", files: text };
}

/** Reads flat config `text` from file `name`. Throws `LintConfigFormatError` when it does not parse or exports no config. */
export function readEslintFlatConfig(text: string, name: string): FlatConfigRead {
  const { parse } = loadParser();
  let program: t.Program;
  try {
    program = parse(text, {
      sourceType: "unambiguous",
      plugins: /\.[cm]?ts$/u.test(name) ? ["typescript"] : [],
      errorRecovery: false,
    }).program;
  } catch (error) {
    throw new LintConfigFormatError(`it does not parse: ${(error as Error).message}`);
  }
  return new FlatConfigReader(text, program).read();
}

class FlatConfigReader {
  readonly #consts = new Map<string, t.Expression>();
  /** Every top-level const of the file, exported ones included: none may be handed to a call (`#nonData`). */
  readonly #locals = new Set<string>();
  /** Local name → the module it is imported from and the name imported (`default`, `*` or the export). */
  readonly #imports = new Map<string, { readonly module: string; readonly imported: string }>();
  readonly #entries: Entry[] = [];
  readonly #unread: string[] = [];
  readonly #notes: string[] = [];
  #steps = 0;
  #values = 0;

  constructor(readonly text: string, readonly program: t.Program) {}

  read(): FlatConfigRead {
    this.#bindings();
    const exported = this.#exported();
    this.#refuseCode(exported);
    this.#elements(exported, { kind: "all" }, 0);
    if (this.#unread.length > 0) {
      this.#notes.push(`the rules of ${this.#unread.join(", ")} are not read: Rerail does not run code to find them.`);
    }
    return { entries: this.#entries, unread: this.#unread, notes: this.#notes };
  }

  /** The file's imports and top-level consts. */
  #bindings(): void {
    for (const statement of this.program.body) {
      if (statement.type === "ImportDeclaration") this.#import(statement);
      else if (statement.type === "VariableDeclaration") this.#declare(statement, false);
      else if (statement.type === "ExportNamedDeclaration" && statement.declaration?.type === "VariableDeclaration") this.#declare(statement.declaration, true);
    }
  }

  /** What `statement` exports as the config, if it exports one: `export default`, `export =` or `module.exports = …`. */
  #exportOf(statement: t.Statement): t.Expression | null | undefined {
    if (statement.type === "ExportDefaultDeclaration") return isExpression(statement.declaration) ? statement.declaration : null;
    if (statement.type === "TSExportAssignment") return statement.expression;
    return statement.type === "ExpressionStatement" ? this.#moduleExports(statement.expression) : undefined;
  }

  /** The config the file exports: the last `export default` or `module.exports = …`; throws where there is none, or both kinds. */
  #exported(): t.Expression {
    // An `export default` of a declaration exports no config Rerail reads (`null`); a later export replaces it.
    const exports = this.program.body.map(statement => this.#exportOf(statement)).filter(found => found !== undefined);
    const exported = exports.at(-1);
    if (exported == null) throw new LintConfigFormatError("it exports no config Rerail can find (export default or module.exports)");
    const esm = this.program.body.some(statement => statement.type === "ExportDefaultDeclaration" || statement.type === "TSExportAssignment");
    const cjs = this.program.body.some(statement => statement.type === "ExpressionStatement" && this.#moduleExports(statement.expression) !== undefined);
    if (esm && cjs) throw new LintConfigFormatError("it exports its config both as a module and through module.exports");
    return exported;
  }

  /** Throws where the file runs code, or exports an object that may hold another config. */
  #refuseCode(exported: t.Expression): void {
    const code = this.#code();
    if (code !== undefined) {
      const line = this.text.slice(0, code.start!).split("\n").length;
      throw new LintConfigFormatError(`it runs code Rerail does not read as data (line ${line}: ${this.#source(code)})`);
    }
    // ESLint loads a TypeScript config through Jiti and takes `config.default ?? config`: an exported object that may
    // hold `default` may be read as that instead.
    const top = this.#unwrap(exported, 0);
    if (top.type === "ObjectExpression"
      && top.properties.some(property => property.type === "SpreadElement" || keyName(property.key, property.computed) === "default")) {
      throw new LintConfigFormatError("it exports an object that may hold a `default` config, which ESLint's loader can take instead");
    }
  }

  /** The value of `module.exports = <value>` where `module` is CommonJS's own, not a binding of the file. */
  #moduleExports(expression: t.Expression): t.Expression | undefined {
    return this.#locals.has("module") || this.#imports.has("module") ? undefined : moduleExports(expression);
  }

  /** The first statement or expression of the file that is code rather than data (see the module comment). */
  #code(): t.Node | undefined {
    for (const statement of this.program.body) {
      const code = this.#statementCode(statement);
      if (code !== undefined) return code;
    }
    return undefined;
  }

  #statementCode(statement: t.Statement): t.Node | undefined {
    switch (statement.type) {
      case "ImportDeclaration":
      case "EmptyStatement":
      case "TSTypeAliasDeclaration":
      case "TSInterfaceDeclaration":
      case "TSDeclareFunction":
        return undefined;
      case "ExportAllDeclaration":
        // A module named by path may export `then` (see below), which a package is trusted not to.
        return isPathModule(statement.source.value) ? statement : undefined;
      case "TSModuleDeclaration":
        return statement.declare === true ? undefined : statement;
      case "VariableDeclaration":
        return this.#declarationCode(statement);
      case "ExportNamedDeclaration":
        return this.#exportNamedCode(statement);
      case "ExportDefaultDeclaration":
        return isExpression(statement.declaration) ? this.#nonData(statement.declaration, false) : statement.declaration;
      case "TSExportAssignment":
        return this.#nonData(statement.expression, false);
      case "ExpressionStatement": {
        const value = this.#moduleExports(statement.expression);
        return value === undefined ? statement : this.#nonData(value, false);
      }
      default:
        return statement;
    }
  }

  #declarationCode(statement: t.VariableDeclaration): t.Node | undefined {
    // A `declare const` is erased before the file runs: its name is the global of that name, not its initializer.
    if (statement.kind !== "const" || statement.declare === true) return statement;
    for (const declarator of statement.declarations) {
      const code = this.#declaratorCode(declarator);
      if (code !== undefined) return code;
    }
    return undefined;
  }

  #declaratorCode(declarator: t.VariableDeclarator): t.Node | undefined {
    return declarator.id.type !== "Identifier" || declarator.init == null ? declarator : this.#nonData(declarator.init, false);
  }

  #exportNamedCode(statement: t.ExportNamedDeclaration): t.Node | undefined {
    // `import()` calls a module's own `then` export as it loads it, and ESLint loads the config with `import()`.
    if (exportedNames(statement).includes("then")) return statement;
    this.#forgetExportedConsts(statement);
    const declared = statement.declaration;
    if (declaresNoValue(declared)) return undefined;
    return declared.type === "VariableDeclaration" ? this.#declarationCode(declared) : declared;
  }

  /** A const another module can import is not read: that module may change it. */
  #forgetExportedConsts(statement: t.ExportNamedDeclaration): void {
    for (const specifier of statement.specifiers) {
      if (specifier.type === "ExportSpecifier" && specifier.local.type === "Identifier") this.#consts.delete(specifier.local.name);
    }
  }

  /**
   * The first part of expression `node` that is code rather than data, or undefined. `given` is whether the value is
   * handed to a call other than a config helper, which could change any object it is given: any of the file's own
   * bindings there is code. `eval` (which reaches the file's bindings without being handed them), `module` and `exports`
   * (through which a call reaches the exported config) are code anywhere but in the one `module.exports = …` statement.
   */
  #nonData(node: t.Node, given: boolean): t.Node | undefined {
    switch (node.type) {
      case "StringLiteral":
      case "NumericLiteral":
      case "BooleanLiteral":
      case "NullLiteral":
      case "BigIntLiteral":
      case "RegExpLiteral":
      case "MetaProperty":
        return undefined;
      case "Identifier":
        return this.#identifierCode(node, given);
      case "TemplateLiteral":
        return this.#firstNonData(node.expressions, given);
      case "MemberExpression":
      case "OptionalMemberExpression":
        return this.#memberCode(node, given);
      case "ArrayExpression":
        return this.#firstNonData(node.elements, given);
      case "ObjectExpression":
        return this.#objectCode(node, given);
      case "CallExpression":
      case "OptionalCallExpression":
      case "NewExpression":
        return this.#callCode(node, given);
      case "UnaryExpression":
        return node.operator === "delete" ? node : this.#nonData(node.argument, given);
      case "BinaryExpression":
      case "LogicalExpression":
        return this.#firstNonData([node.left, node.right], given);
      case "ConditionalExpression":
        return this.#firstNonData([node.test, node.consequent, node.alternate], given);
      case "TSAsExpression":
      case "TSSatisfiesExpression":
      case "TSNonNullExpression":
      case "TSTypeAssertion":
      case "TSInstantiationExpression":
        return this.#nonData(node.expression, given);
      default:
        return node;
    }
  }

  #firstNonData(nodes: readonly (t.Node | null | undefined)[], inside: boolean): t.Node | undefined {
    for (const child of nodes) {
      if (child == null) continue;
      const code = this.#nonData(child.type === "SpreadElement" ? child.argument : child, inside);
      if (code !== undefined) return code;
    }
    return undefined;
  }

  #identifierCode(node: t.Identifier, given: boolean): t.Node | undefined {
    if (this.#isGlobal(node.name)) return DATA_GLOBALS.has(node.name) ? undefined : node;
    // A code-running Node module is code wherever it is named: a value read from it can run code when coerced.
    const from = this.#imports.get(node.name)?.module;
    if (isCodeModule(from)) return node;
    return given && this.#callMayChange(node.name, from) ? node : undefined;
  }

  /** The file's own binding, or one from a module named by path: that may be the config itself, whose export a call may change. */
  #callMayChange(name: string, from: string | undefined): boolean {
    return this.#locals.has(name) || isPathModuleName(from);
  }

  #memberCode(node: t.MemberExpression | t.OptionalMemberExpression, given: boolean): t.Node | undefined {
    // `.constructor` of any function is `Function`; a key Rerail cannot read may be that one.
    const key = keyName(node.property, node.computed);
    if (key === undefined || REFLECTIVE_KEYS.has(key)) return node;
    // The environment's variables are strings: reading them reaches nothing else of the process.
    if (key === "env" && this.#isGlobalProcess(node.object)) return undefined;
    return this.#nonData(node.object, given);
  }

  #objectCode(node: t.ObjectExpression, given: boolean): t.Node | undefined {
    for (const property of node.properties) {
      const code = this.#propertyCode(property, given);
      if (code !== undefined) return code;
    }
    return undefined;
  }

  #propertyCode(property: t.ObjectExpression["properties"][number], given: boolean): t.Node | undefined {
    if (property.type === "ObjectMethod") return property;
    if (property.type === "SpreadElement") return this.#nonData(property.argument, given);
    // JavaScript calls `toString`, `valueOf` or `toJSON` itself when it turns the object into a string or JSON.
    const key = keyName(property.key, property.computed);
    if (key === undefined || REFLECTIVE_KEYS.has(key) || COERCION_KEYS.has(key)) return property;
    return this.#nonData(property.value, given);
  }

  #callCode(node: t.CallExpression | t.OptionalCallExpression | t.NewExpression, given: boolean): t.Node | undefined {
    const helper = this.#isConfigHelperCall(node);
    // Only calls into the packages the file imports and `require("<package>")`, `new URL(…)`, `process.cwd()` and
    // `import.meta.resolve(…)`: a global such as `Object`, `Reflect`, `Function` or `globalThis.eval` can reach
    // anything.
    const trusted = helper ? "package" : this.#trustedCall(node);
    if (trusted === undefined) return node;
    if (given && requiresPathModule(node)) return node;
    return this.#calleeCode(node, trusted) ?? this.#firstNonData(node.arguments, given || !helper);
  }

  #isConfigHelperCall(node: t.CallExpression | t.OptionalCallExpression | t.NewExpression): boolean {
    return node.type === "CallExpression" && this.#isConfigHelper(node.callee);
  }

  #calleeCode(node: t.CallExpression | t.OptionalCallExpression | t.NewExpression, trusted: "global" | "package"): t.Node | undefined {
    return trusted === "global" ? undefined : this.#nonData(node.callee, true);
  }

  #import(statement: t.ImportDeclaration): void {
    // A type-only import is erased before the file runs: its names are the globals of those names there.
    if (statement.importKind === "type" || statement.importKind === "typeof") return;
    for (const specifier of statement.specifiers) {
      if (specifier.type === "ImportSpecifier" && (specifier.importKind === "type" || specifier.importKind === "typeof")) continue;
      const imported = specifier.type === "ImportSpecifier"
        ? (specifier.imported.type === "Identifier" ? specifier.imported.name : specifier.imported.value)
        : specifier.type === "ImportDefaultSpecifier" ? "default" : "*";
      this.#imports.set(specifier.local.name, { module: statement.source.value, imported });
    }
  }

  #declare(declaration: t.VariableDeclaration, exported: boolean): void {
    // Only a const keeps its value; a `let` or `var` may be reassigned anywhere, so it is not read.
    if (declaration.kind !== "const") return;
    for (const declarator of declaration.declarations) {
      if (declarator.id.type !== "Identifier" || declarator.init == null) continue;
      if (declarator.init.type === "CallExpression" && isRequire(declarator.init)) {
        // A required module is an import: Rerail takes it to behave as documented, as it does `import`s.
        this.#imports.set(declarator.id.name, { module: requiredModule(declarator.init), imported: "default" });
        continue;
      }
      this.#locals.add(declarator.id.name);
      // An exported const is not read: a module that imports it may change it.
      if (!exported) this.#consts.set(declarator.id.name, declarator.init);
    }
  }

  /** How `node` is written in the file, on one line and cut short. The parser gives every node its place in the text. */
  #source(node: t.Node): string {
    const text = this.text.slice(node.start!, node.end!).replace(/\s+/gu, " ");
    return text.length > MAX_SOURCE_TEXT ? `${text.slice(0, MAX_SOURCE_TEXT - 1)}…` : text;
  }

  /** One step into a value; throws once the file takes more than `MAX_STEPS`. */
  #step(): void {
    this.#steps += 1;
    if (this.#steps > MAX_STEPS) throw new LintConfigFormatError(`it takes more than ${MAX_STEPS} steps to read`);
  }

  /** `count` more JSON values built; throws once the file builds more than `MAX_VALUES`. */
  #built(count: number): void {
    this.#values += count;
    if (this.#values > MAX_VALUES) throw new LintConfigFormatError(`it builds more than ${MAX_VALUES} values`);
  }

  /** `node` without TypeScript's type wrappers, and a same-file `const` it names replaced by its value. */
  #unwrap(node: t.Expression, depth: number): t.Expression {
    let current = node;
    for (let step = depth; step < MAX_DEPTH; step += 1) {
      const inner = isTypeWrapper(current) ? current.expression : current.type === "Identifier" ? this.#consts.get(current.name) : undefined;
      if (inner === undefined) return current;
      current = inner;
    }
    return current;
  }

  /** The config objects `node` holds, in order, as ESLint flattens a config array. */
  #elements(node: t.Expression, scope: FlatConfigScope, depth: number): void {
    this.#step();
    const value = this.#unwrap(node, depth);
    if (depth > MAX_DEPTH) throw new LintConfigFormatError("it nests deeper than Rerail reads");
    const nested = this.#nestedElements(value);
    if (nested !== undefined) for (const element of nested) this.#elements(element, scope, depth + 1);
    else if (value.type === "ObjectExpression") this.#object(value, scope, depth + 1);
    else this.#notRead(node, scope, depth);
  }

  /** The elements an array or a config helper's call holds, spreads taken apart; undefined for any other value. */
  #nestedElements(value: t.Expression): t.Expression[] | undefined {
    const items = value.type === "ArrayExpression" ? value.elements
      : value.type === "CallExpression" && this.#isConfigHelper(value.callee) ? value.arguments
      : undefined;
    // The parser is given no plugin that writes an argument placeholder (`f(?)`), so every other item is an expression.
    return items?.flatMap(item => {
      if (item === null) return [];
      return [item.type === "SpreadElement" ? item.argument : item as t.Expression];
    });
  }

  /**
   * Whether `node` is a value from a module the file imports: an imported config, a member of one, what a call into
   * an imported package returns, or a choice between such values (or nothing: `cond ? shared : []`). Rerail names
   * such a value as not read; it cannot read it without the module.
   */
  #fromImport(node: t.Expression, depth: number): boolean {
    this.#step();
    let root: t.Node = this.#unwrap(node, depth);
    if (root.type === "ConditionalExpression") {
      const branches = [root.consequent, root.alternate].map(branch => this.#unwrap(branch, depth + 1));
      const empty = (branch: t.Expression): boolean => (branch.type === "ArrayExpression" && branch.elements.length === 0)
        || (branch.type === "ObjectExpression" && branch.properties.length === 0);
      return branches.every(branch => empty(branch) || this.#fromImport(branch, depth + 1)) && !branches.every(empty);
    }
    if ((root.type === "CallExpression" || root.type === "OptionalCallExpression") && this.#trustedCall(root) === "package") root = root.callee;
    while (root.type === "MemberExpression" || root.type === "OptionalMemberExpression") root = root.object;
    return root.type === "Identifier" && this.#imports.has(root.name);
  }

  /**
   * What Rerail does not read in place of `node`: a recognised shared config contributes its entries in place
   * (`#addPreset`); any other value from an imported module is named as not read (see `#unreadable`); anything of the
   * file's own that Rerail cannot follow makes the file not statically readable.
   */
  #notRead(node: t.Expression, scope: FlatConfigScope, depth: number, own?: Map<string, Entry>): void {
    if (!this.#fromImport(node, depth)) throw new LintConfigFormatError(`Rerail cannot read ${this.#source(node)} as data`);
    const preset = this.#presetOf(node, depth);
    if (preset === undefined) this.#unreadable(this.#source(node), own);
    else if (own === undefined) this.#addPreset(preset, scope);
    else for (const entry of preset.entries) this.#addPresetRules(preset.text, entry.rules, scope, own);
  }

  /**
   * The binding a member path (`js.configs.recommended`) hangs off, with the path's keys. A file read as data names its
   * members by literal keys only (`#memberCode`), and `super` does not parse outside a method, which is code.
   */
  #memberPath(node: t.Expression, depth: number): { readonly root: t.Expression; readonly path: string[] } {
    const path: string[] = [];
    let current = this.#unwrap(node, depth);
    while (current.type === "MemberExpression" || current.type === "OptionalMemberExpression") {
      path.unshift(keyName(current.property, current.computed)!);
      current = this.#unwrap(current.object as t.Expression, depth);
    }
    return { root: current, path };
  }

  /**
   * The recognised shared config `node` is, with `member` after it: a member path of a binding the file imports that
   * names one of `eslint-presets.ts`'s presets.
   */
  #presetOf(node: t.Expression, depth: number, member: readonly string[] = []): Preset | undefined {
    const { root, path } = this.#memberPath(node, depth);
    const from = root.type === "Identifier" ? this.#imports.get(root.name) : undefined;
    const entries = from === undefined ? undefined : flatPreset(from.module, from.imported, [...path, ...member]);
    return entries === undefined ? undefined : { text: this.#source(node), entries };
  }

  /** A shared config's entries, in place, each for the files of the element it stands in and its own `files`. */
  #addPreset(preset: Preset, scope: FlatConfigScope): void {
    for (const entry of preset.entries) {
      const own = new Map<string, Entry>();
      this.#addPresetRules(preset.text, entry.rules, presetScope(entry, scope), own);
      for (const added of own.values()) this.#entries.push(added);
    }
  }

  /** A shared config's rules into the `rules` object being read, as its entries. */
  #addPresetRules(text: string, rules: Readonly<Record<string, unknown>>, scope: FlatConfigScope, own: Map<string, Entry>): void {
    this.#built(Object.keys(rules).length);
    for (const [key, value] of Object.entries(rules)) {
      own.delete(key);
      own.set(key, { key, scope, value: { ok: true, value }, preset: text });
    }
  }

  /**
   * Something Rerail does not read, named in the notes. It may set any rule, so every entry before it (and in `own`,
   * the entries of the `rules` object being read) is marked as one it may replace.
   */
  #unreadable(text: string, own: ReadonlyMap<string, Entry> = new Map()): void {
    this.#unread.push(text);
    for (const entry of [...this.#entries, ...own.values()]) entry.shadowedBy ??= text;
  }

  /**
   * Whether `call` calls into a package the file imports or requires, through any member path; or is
   * `require("<module>")`, `new URL(…)`, `process.cwd()` or `import.meta.resolve(…)` (globals nothing in the file has
   * replaced). A module the file names by path (`./x`, `/x`) may be the config itself, and a Node module that runs code
   * or reaches the module system (`node:vm`, `node:module`, ...) can change anything: calls into them are code.
   */
  #trustedCall(call: t.CallExpression | t.OptionalCallExpression | t.NewExpression): "global" | "package" | undefined {
    if (this.#isTrustedGlobalCall(call)) return "global";
    const root = memberRoot(call.callee);
    if (root.type !== "Identifier") return undefined;
    return isPackageModule(this.#imports.get(root.name)?.module) ? "package" : undefined;
  }

  #isTrustedGlobalCall(call: t.CallExpression | t.OptionalCallExpression | t.NewExpression): boolean {
    return this.#isGlobalRequire(call) || this.#isGlobalNewUrl(call) || this.#isGlobalProcessCwd(call) || isImportMetaResolve(call);
  }

  #isGlobalRequire(call: t.CallExpression | t.OptionalCallExpression | t.NewExpression): boolean {
    return call.type === "CallExpression" && isRequire(call) && this.#isGlobal("require");
  }

  #isGlobalNewUrl(call: t.CallExpression | t.OptionalCallExpression | t.NewExpression): boolean {
    return call.type === "NewExpression" && call.callee.type === "Identifier" && call.callee.name === "URL" && this.#isGlobal("URL");
  }

  #isGlobalProcessCwd(call: t.CallExpression | t.OptionalCallExpression | t.NewExpression): boolean {
    const method = plainMethodCall(call);
    return method !== undefined && this.#isGlobalProcess(method.object) && method.name === "cwd" && call.arguments.length === 0;
  }

  /** The `process` of the global scope: the file neither declares nor imports one. */
  #isGlobalProcess(node: t.Node): boolean {
    return node.type === "Identifier" && node.name === "process" && this.#isGlobal("process");
  }

  /** Whether `name` is the global of that name: the file neither declares nor imports it. */
  #isGlobal(name: string): boolean {
    return !this.#locals.has(name) && !this.#imports.has(name);
  }

  /** `defineConfig` imported from ESLint, or typescript-eslint's `config`: both concatenate their arguments. */
  #isConfigHelper(callee: t.Node): boolean {
    if (callee.type === "Identifier") {
      const from = this.#imports.get(callee.name);
      return from !== undefined && DEFINE_CONFIG_MODULES.has(from.module) && from.imported === "defineConfig";
    }
    if (callee.type === "MemberExpression" && !callee.computed && callee.object.type === "Identifier" && callee.property.type === "Identifier") {
      const from = this.#imports.get(callee.object.name);
      return from !== undefined && TYPESCRIPT_ESLINT_MODULES.has(from.module) && callee.property.name === "config"
        && (from.imported === "default" || from.imported === "*");
    }
    return false;
  }

  /**
   * A config object's properties by name; undefined where a spread imported config Rerail does not recognise hides what
   * they are. A spread recognised shared config object gives its `files`, `ignores` and `rules`.
   */
  #properties(object: t.ObjectExpression, depth: number): Map<string, Property> | undefined {
    this.#step();
    const properties = new Map<string, Property>();
    for (const property of object.properties) {
      const added = property.type === "SpreadElement" ? this.#spreadProperties(property.argument, depth) : new Map([dataProperty(property)]);
      if (added === undefined) return undefined;
      for (const [name, value] of added) properties.set(name, value);
    }
    return properties;
  }

  /** The properties a spread in a config object adds; undefined where it hides them (an imported config Rerail does not recognise). */
  #spreadProperties(argument: t.Expression, depth: number): Map<string, Property> | undefined {
    const spread = this.#unwrap(argument, depth);
    if (spread.type === "ObjectExpression") return this.#properties(spread, depth + 1);
    // A spread imported config hides what the object holds; one of the file's own Rerail cannot follow is code.
    if (!this.#fromImport(argument, depth)) throw new LintConfigFormatError(`Rerail cannot read ${this.#source(argument)} as data`);
    // Only a config object has a `rules` member preset; spreading an array adds no config key.
    const preset = this.#presetOf(argument, depth, ["rules"]) === undefined ? undefined : this.#presetOf(argument, depth);
    const entry = preset?.entries[0];
    if (entry === undefined) return undefined;
    return new Map(Object.entries(entry).map(([name, json]) => [name, new PresetValue(preset!.text, json)]));
  }

  #object(object: t.ObjectExpression, inherited: FlatConfigScope, depth: number): void {
    const properties = this.#properties(object, depth);
    if (properties === undefined) {
      this.#unreadable(this.#source(object));
      return;
    }
    const keys = [...properties.keys()].filter(key => key !== "name");
    // An object with nothing but `ignores` ignores those files everywhere; it sets no rule.
    if (keys.length === 1 && keys[0] === "ignores") return;
    const scope = this.#scope(properties, inherited, depth);
    const extended = properties.get("extends");
    if (extended !== undefined && !(extended instanceof PresetValue)) this.#elements(extended, scope, depth + 1);
    this.#objectRules(properties.get("rules"), scope, depth);
  }

  /** A config object's `rules`: written in the file, or a recognised shared config object's, spread into it. */
  #objectRules(rules: Property | undefined, scope: FlatConfigScope, depth: number): void {
    if (rules instanceof PresetValue) this.#addPreset({ text: rules.preset, entries: [{ rules: rules.json as Record<string, unknown> }] }, scope);
    else if (rules !== undefined) this.#rules(rules, scope, depth + 1);
  }

  /** A config object's `files` as written (`text`) and as JSON, where Rerail can read it. */
  #files(files: Property | undefined, depth: number): { readonly text: string; readonly patterns: unknown } {
    if (files === undefined) return { text: "some files", patterns: undefined };
    if (files instanceof PresetValue) return { text: JSON.stringify(files.json), patterns: files.json };
    const patterns = this.#resolve(files, depth);
    return { text: this.#source(files), patterns: patterns.ok ? patterns.value : undefined };
  }

  #scope(properties: ReadonlyMap<string, Property>, inherited: FlatConfigScope, depth: number): FlatConfigScope {
    const files = properties.get("files");
    const narrowed = properties.has("ignores") || properties.has("basePath");
    if (files === undefined && !narrowed) return inherited;
    const read = this.#files(files, depth);
    // Within an `extends` of a limited object the files are the intersection; Rerail maps neither.
    return inherited.kind !== "all" || narrowed ? { kind: "some", files: read.text } : filesScope(read.patterns, read.text);
  }

  /**
   * The entries of one config object's `rules`, read as JavaScript builds the object: a later key, spread or written
   * out, replaces an earlier one whole. ESLint's merging (a severity keeping earlier options) is between objects only.
   */
  #rules(node: t.Expression, scope: FlatConfigScope, depth: number): void {
    const own = new Map<string, Entry>();
    this.#gatherRules(node, scope, depth, own);
    for (const entry of own.values()) this.#entries.push(entry);
  }

  #gatherRules(node: t.Expression, scope: FlatConfigScope, depth: number, own: Map<string, Entry>): void {
    this.#step();
    if (depth > MAX_DEPTH) throw new LintConfigFormatError("it nests deeper than Rerail reads");
    const rules = this.#unwrap(node, depth);
    if (rules.type !== "ObjectExpression") {
      this.#notRead(node, scope, depth, own);
      return;
    }
    for (const property of rules.properties) {
      if (property.type === "SpreadElement") this.#gatherRules(property.argument, scope, depth + 1, own);
      else this.#gatherRule(property, scope, depth, own);
    }
  }

  /** One rule entry written out in a `rules` object. */
  #gatherRule(property: t.ObjectProperty | t.ObjectMethod, scope: FlatConfigScope, depth: number, own: Map<string, Entry>): void {
    const [key, node] = dataProperty(property);
    const value = this.#resolve(node, depth + 1);
    own.set(key, { key, scope, value: value.ok ? { ok: true, value: value.value } : { ok: false, why: value.why } });
  }

  /** `node` as JSON where it is literal, through same-file `const` bindings and spreads; why not otherwise. */
  #resolve(node: t.Expression, depth: number): Resolved {
    this.#step();
    if (depth > MAX_DEPTH) return { ok: false, why: "it nests too deep" };
    const value = this.#unwrap(node, depth);
    switch (value.type) {
      case "StringLiteral":
      case "NumericLiteral":
      case "BooleanLiteral":
        return { ok: true, value: value.value };
      case "NullLiteral":
        return { ok: true, value: null };
      case "TemplateLiteral":
        return this.#resolveTemplate(value);
      case "UnaryExpression":
        return this.#resolveUnary(value);
      case "ArrayExpression":
        return this.#resolveArray(value, depth);
      case "ObjectExpression":
        return this.#resolveObject(value, depth);
      default:
        return this.#unreadValue(value);
    }
  }

  #unreadValue(value: t.Expression): Unresolved {
    return { ok: false, why: `it is ${this.#source(value)}, which Rerail does not run` };
  }

  /** A template literal without `${…}` is its one string; an untagged template's escapes are valid, so it has one. */
  #resolveTemplate(value: t.TemplateLiteral): Resolved {
    return value.expressions.length === 0 ? { ok: true, value: value.quasis[0]!.value.cooked! } : this.#unreadValue(value);
  }

  #resolveUnary(value: t.UnaryExpression): Resolved {
    if ((value.operator !== "-" && value.operator !== "+") || value.argument.type !== "NumericLiteral") return this.#unreadValue(value);
    return { ok: true, value: value.operator === "-" ? -value.argument.value : value.argument.value };
  }

  #resolveArray(value: t.ArrayExpression, depth: number): Resolved {
    const items: Json[] = [];
    for (const element of value.elements) {
      const added = this.#elementItems(value, element, depth);
      if (!added.ok) return added;
      this.#built(added.value.length);
      for (const item of added.value) items.push(item);
    }
    return { ok: true, value: items };
  }

  /** What one element adds to an array: its value, or the items of a spread array. */
  #elementItems(array: t.ArrayExpression, element: t.ArrayExpression["elements"][number], depth: number): { readonly ok: true; readonly value: Json[] } | Unresolved {
    if (element === null) return this.#unreadValue(array);
    if (element.type !== "SpreadElement") {
      const inner = this.#resolve(element, depth + 1);
      return inner.ok ? { ok: true, value: [inner.value] } : inner;
    }
    const inner = this.#resolve(element.argument, depth + 1);
    if (!inner.ok) return inner;
    return Array.isArray(inner.value) ? { ok: true, value: inner.value } : this.#unreadValue(array);
  }

  #resolveObject(value: t.ObjectExpression, depth: number): Resolved {
    const object: { [key: string]: Json } = {};
    for (const property of value.properties) {
      const unresolved = property.type === "SpreadElement"
        ? this.#spreadInto(object, value, property, depth)
        : this.#propertyInto(object, property, depth);
      if (unresolved !== undefined) return unresolved;
    }
    return { ok: true, value: object };
  }

  #spreadInto(object: { [key: string]: Json }, value: t.ObjectExpression, property: t.SpreadElement, depth: number): Unresolved | undefined {
    const inner = this.#resolve(property.argument, depth + 1);
    if (!inner.ok) return inner;
    if (inner.value === null || typeof inner.value !== "object" || Array.isArray(inner.value)) return this.#unreadValue(value);
    this.#built(Object.keys(inner.value).length);
    Object.assign(object, inner.value);
    return undefined;
  }

  #propertyInto(object: { [key: string]: Json }, property: t.ObjectProperty | t.ObjectMethod, depth: number): Unresolved | undefined {
    const [key, node] = dataProperty(property);
    const inner = this.#resolve(node, depth + 1);
    if (!inner.ok) return inner;
    this.#built(1);
    Object.defineProperty(object, key, { value: inner.value, enumerable: true, writable: true, configurable: true });
    return undefined;
  }
}

/** What a member path `a.b.c` hangs off: `a`. */
function memberRoot(node: t.Node): t.Node {
  let root = node;
  while (root.type === "MemberExpression" || root.type === "OptionalMemberExpression") root = root.object;
  return root;
}

const isCodeModule = (module: string | undefined): boolean => module !== undefined && CODE_MODULES.has(module.replace(/^node:/u, ""));

const isPathModuleName = (module: string | undefined): boolean => module !== undefined && isPathModule(module);

/** A module named by package, which is trusted to behave as documented: not one named by path, nor one that runs code. */
function isPackageModule(module: string | undefined): boolean {
  return module !== undefined && !isPathModule(module) && !isCodeModule(module);
}

/** The object and method name of a plain `object.method(…)` call. */
function plainMethodCall(call: t.CallExpression | t.OptionalCallExpression | t.NewExpression): { readonly object: t.Node; readonly name: string } | undefined {
  const callee = call.callee;
  if (call.type !== "CallExpression" || callee.type !== "MemberExpression" || callee.computed || callee.property.type !== "Identifier") return undefined;
  return { object: callee.object, name: callee.property.name };
}

function isImportMetaResolve(call: t.CallExpression | t.OptionalCallExpression | t.NewExpression): boolean {
  const method = plainMethodCall(call);
  return method?.object.type === "MetaProperty" && method.object.meta.name === "import" && method.name === "resolve";
}

function requiresPathModule(node: t.CallExpression | t.OptionalCallExpression | t.NewExpression): boolean {
  return node.type === "CallExpression" && isRequire(node) && isPathModule(requiredModule(node));
}

/** The names an `export { … }` or `export const …` gives; undefined for a destructured declarator. */
function exportedNames(statement: t.ExportNamedDeclaration): (string | undefined)[] {
  return statement.declaration?.type === "VariableDeclaration"
    ? statement.declaration.declarations.map(declarator => (declarator.id.type === "Identifier" ? declarator.id.name : undefined))
    : statement.specifiers.map(specifier => (specifier.exported.type === "Identifier" ? specifier.exported.name : specifier.exported.value));
}

function declaresNoValue(declared: t.ExportNamedDeclaration["declaration"]): declared is null | undefined | t.TSTypeAliasDeclaration | t.TSInterfaceDeclaration {
  return declared == null || declared.type === "TSTypeAliasDeclaration" || declared.type === "TSInterfaceDeclaration";
}

/** Whether `export default` exports an expression rather than a declaration (`function`, `class`). */
function isExpression(declared: t.ExportDefaultDeclaration["declaration"]): declared is t.Expression {
  return !declared.type.endsWith("Declaration");
}

/**
 * A property written out in an object of a file read as data, as its key and value. `#propertyCode` lets no method
 * through, nor any key but a name or a literal; and a property of an object literal holds an expression.
 */
function dataProperty(property: t.ObjectProperty | t.ObjectMethod): [string, t.Expression] {
  return [keyName(property.key, property.computed)!, (property as t.ObjectProperty).value as t.Expression];
}

/** The value of `module.exports = <value>`, or undefined for any other expression. */
function moduleExports(expression: t.Expression): t.Expression | undefined {
  if (expression.type !== "AssignmentExpression" || expression.operator !== "=") return undefined;
  const target = expression.left;
  const isModuleExports = target.type === "MemberExpression" && !target.computed && target.object.type === "Identifier"
    && target.object.name === "module" && target.property.type === "Identifier" && target.property.name === "exports";
  return isModuleExports ? expression.right : undefined;
}

function isRequire(call: t.CallExpression): boolean {
  return call.callee.type === "Identifier" && call.callee.name === "require" && call.arguments.length === 1 && call.arguments[0]!.type === "StringLiteral";
}

function requiredModule(call: t.CallExpression): string {
  return (call.arguments[0] as t.StringLiteral).value;
}
