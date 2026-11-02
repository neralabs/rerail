/**
 * ESLint's flat config (`eslint.config.{js,mjs,cjs,ts,mts,cts}`) read as data: the file is parsed, never run or
 * imported. A file is read only when it is data throughout: imports, top-level `const`s, the export, and type-only
 * declarations, built from literals, arrays, objects without methods, spreads, reads of members, and calls that are given
 * no local `const` (a config helper excepted); anything else (a function, a statement that changes something, a
 * namespace, a loop) is code, and the file is not read. Nothing in such a file changes a `const` once set, so the rule
 * entries of the exported config array are read through same-file `const` bindings, array and object spreads,
 * `defineConfig(...)` (from `eslint/config` or `@eslint/config-helpers`), typescript-eslint's `config(...)` and
 * `extends`. Rerail takes those helpers, and the modules the file imports, to do what their documentation says; it does
 * not read them. What the file takes from them (a shared config such as `js.configs.recommended`, an imported value,
 * a call's result) is not evaluated: a rule entry whose value it decides is reported as unresolved, and a config element
 * it is is named in a note.
 */
import { createRequire } from "node:module";
import type * as BabelParser from "@babel/parser";
import type * as t from "@babel/types";
import { LintConfigFormatError } from "./lint-config-formats.js";

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
}

type Entry = { -readonly [K in keyof FlatConfigEntry]: FlatConfigEntry[K] };

export interface FlatConfigRead {
  readonly entries: readonly FlatConfigEntry[];
  /** What the config holds that is not read, for the person. */
  readonly notes: readonly string[];
}

type Json = null | boolean | number | string | Json[] | { [key: string]: Json };
type Resolved = { readonly ok: true; readonly value: Json } | { readonly ok: false; readonly why: string };

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
    // The last `export default` or `module.exports = …` is the config.
    let exported: t.Expression | undefined;
    for (const statement of this.program.body) {
      if (statement.type === "ImportDeclaration") this.#import(statement);
      else if (statement.type === "VariableDeclaration") this.#declare(statement, false);
      else if (statement.type === "ExportNamedDeclaration" && statement.declaration?.type === "VariableDeclaration") this.#declare(statement.declaration, true);
    }
    for (const statement of this.program.body) {
      if (statement.type === "ExportDefaultDeclaration") exported = isExpression(statement.declaration) ? statement.declaration : undefined;
      else if (statement.type === "TSExportAssignment") exported = statement.expression;
      else if (statement.type === "ExpressionStatement") exported = this.#moduleExports(statement.expression) ?? exported;
    }
    if (exported === undefined) throw new LintConfigFormatError("it exports no config Rerail can find (export default or module.exports)");
    const esm = this.program.body.some(statement => statement.type === "ExportDefaultDeclaration" || statement.type === "TSExportAssignment");
    const cjs = this.program.body.some(statement => statement.type === "ExpressionStatement" && this.#moduleExports(statement.expression) !== undefined);
    if (esm && cjs) throw new LintConfigFormatError("it exports its config both as a module and through module.exports");
    const code = this.#code();
    if (code !== undefined) {
      const line = this.text.slice(0, code.start ?? 0).split("\n").length;
      throw new LintConfigFormatError(`it runs code Rerail does not read as data (line ${line}: ${this.#source(code)})`);
    }
    // ESLint loads a TypeScript config through Jiti and takes `config.default ?? config`: an exported object that may
    // hold `default` may be read as that instead.
    const top = this.#unwrap(exported, 0);
    if (top.type === "ObjectExpression"
      && top.properties.some(property => property.type === "SpreadElement" || keyName(property.key, property.computed) === "default")) {
      throw new LintConfigFormatError("it exports an object that may hold a `default` config, which ESLint's loader can take instead");
    }
    this.#elements(exported, { kind: "all" }, 0);
    if (this.#unread.length > 0) {
      this.#notes.push(`the rules of ${this.#unread.join(", ")} are not read: Rerail does not run code to find them.`);
    }
    return { entries: this.#entries, notes: this.#notes };
  }

  /** The value of `module.exports = <value>` where `module` is CommonJS's own, not a binding of the file. */
  #moduleExports(expression: t.Expression): t.Expression | undefined {
    return this.#locals.has("module") || this.#imports.has("module") ? undefined : moduleExports(expression);
  }

  /** The first statement or expression of the file that is code rather than data (see the module comment). */
  #code(): t.Node | undefined {
    const declaration = (statement: t.VariableDeclaration): t.Node | undefined => {
      // A `declare const` is erased before the file runs: its name is the global of that name, not its initializer.
      if (statement.kind !== "const" || statement.declare === true) return statement;
      for (const declarator of statement.declarations) {
        if (declarator.id.type !== "Identifier" || declarator.init == null) return declarator;
        const code = this.#nonData(declarator.init, false);
        if (code !== undefined) return code;
      }
      return undefined;
    };
    for (const statement of this.program.body) {
      switch (statement.type) {
        case "ImportDeclaration":
          continue;
        case "ExportAllDeclaration":
          // A module named by path may export `then` (see below), which a package is trusted not to.
          if (isPathModule(statement.source.value)) return statement;
          continue;
        case "EmptyStatement":
        case "TSTypeAliasDeclaration":
        case "TSInterfaceDeclaration":
        case "TSDeclareFunction":
          continue;
        case "TSModuleDeclaration":
          if (statement.declare === true) continue;
          return statement;
        case "VariableDeclaration": {
          const code = declaration(statement);
          if (code !== undefined) return code;
          continue;
        }
        case "ExportNamedDeclaration": {
          // `import()` calls a module's own `then` export as it loads it, and ESLint loads the config with `import()`.
          const exported = statement.declaration?.type === "VariableDeclaration"
            ? statement.declaration.declarations.map(declarator => (declarator.id.type === "Identifier" ? declarator.id.name : undefined))
            : statement.specifiers.map(specifier => (specifier.exported.type === "Identifier" ? specifier.exported.name : specifier.exported.value));
          if (exported.includes("then")) return statement;
          // A const another module can import is not read: that module may change it.
          for (const specifier of statement.specifiers) {
            if (specifier.type === "ExportSpecifier" && specifier.local.type === "Identifier") this.#consts.delete(specifier.local.name);
          }
          const declared = statement.declaration;
          if (declared == null || declared.type === "TSTypeAliasDeclaration" || declared.type === "TSInterfaceDeclaration") continue;
          if (declared.type !== "VariableDeclaration") return declared;
          const code = declaration(declared);
          if (code !== undefined) return code;
          continue;
        }
        case "ExportDefaultDeclaration": {
          if (!isExpression(statement.declaration)) return statement.declaration;
          const code = this.#nonData(statement.declaration, false);
          if (code !== undefined) return code;
          continue;
        }
        case "TSExportAssignment": {
          const code = this.#nonData(statement.expression, false);
          if (code !== undefined) return code;
          continue;
        }
        case "ExpressionStatement": {
          const value = this.#moduleExports(statement.expression);
          if (value === undefined) return statement;
          const code = this.#nonData(value, false);
          if (code !== undefined) return code;
          continue;
        }
        default:
          return statement;
      }
    }
    return undefined;
  }

  /**
   * The first part of expression `node` that is code rather than data, or undefined. `given` is whether the value is
   * handed to a call other than a config helper, which could change any object it is given: any of the file's own
   * bindings there is code. `eval` (which reaches the file's bindings without being handed them), `module` and `exports`
   * (through which a call reaches the exported config) are code anywhere but in the one `module.exports = …` statement.
   */
  #nonData(node: t.Node, given: boolean): t.Node | undefined {
    const all = (nodes: readonly (t.Node | null | undefined)[], inside: boolean): t.Node | undefined => {
      for (const child of nodes) {
        if (child == null) continue;
        const code = this.#nonData(child.type === "SpreadElement" ? child.argument : child, inside);
        if (code !== undefined) return code;
      }
      return undefined;
    };
    switch (node.type) {
      case "StringLiteral":
      case "NumericLiteral":
      case "BooleanLiteral":
      case "NullLiteral":
      case "BigIntLiteral":
      case "RegExpLiteral":
      case "MetaProperty":
        return undefined;
      case "Identifier": {
        if (!this.#locals.has(node.name) && !this.#imports.has(node.name)) return DATA_GLOBALS.has(node.name) ? undefined : node;
        // A code-running Node module is code wherever it is named: a value read from it can run code when coerced.
        const from = this.#imports.get(node.name)?.module;
        if (from !== undefined && CODE_MODULES.has(from.replace(/^node:/u, ""))) return node;
        // A module named by path may be the config itself: handing its values to a call may change the export.
        return given && (this.#locals.has(node.name) || (from !== undefined && isPathModule(from))) ? node : undefined;
      }
      case "TemplateLiteral":
        return all(node.expressions, given);
      case "MemberExpression":
      case "OptionalMemberExpression": {
        // `.constructor` of any function is `Function`; a key Rerail cannot read may be that one.
        const key = keyName(node.property, node.computed);
        if (key === undefined || REFLECTIVE_KEYS.has(key)) return node;
        // The environment's variables are strings: reading them reaches nothing else of the process.
        if (key === "env" && node.object.type === "Identifier" && node.object.name === "process" && this.#isGlobal("process")) return undefined;
        return this.#nonData(node.object, given);
      }
      case "ArrayExpression":
        return all(node.elements, given);
      case "ObjectExpression": {
        for (const property of node.properties) {
          if (property.type === "ObjectMethod") return property;
          if (property.type === "SpreadElement") {
            const code = this.#nonData(property.argument, given);
            if (code !== undefined) return code;
            continue;
          }
          // JavaScript calls `toString`, `valueOf` or `toJSON` itself when it turns the object into a string or JSON.
          const key = keyName(property.key, property.computed);
          if (key === undefined || REFLECTIVE_KEYS.has(key) || COERCION_KEYS.has(key)) return property;
          const code = this.#nonData(property.value, given);
          if (code !== undefined) return code;
        }
        return undefined;
      }
      case "CallExpression":
      case "OptionalCallExpression":
      case "NewExpression": {
        const helper = node.type === "CallExpression" && this.#isConfigHelper(node.callee);
        // Only calls into the packages the file imports and `require("<package>")`, `new URL(…)`, `process.cwd()` and
        // `import.meta.resolve(…)`: a global such as `Object`, `Reflect`, `Function` or `globalThis.eval` can reach
        // anything.
        const trusted = helper ? "package" : this.#trustedCall(node);
        if (trusted === undefined) return node;
        if (given && node.type === "CallExpression" && isRequire(node) && isPathModule(requiredModule(node))) return node;
        return (trusted === "global" ? undefined : this.#nonData(node.callee, true)) ?? all(node.arguments, given || !helper);
      }
      case "UnaryExpression":
        return node.operator === "delete" ? node : this.#nonData(node.argument, given);
      case "BinaryExpression":
      case "LogicalExpression":
        return node.left.type === "PrivateName" ? node : all([node.left, node.right], given);
      case "ConditionalExpression":
        return all([node.test, node.consequent, node.alternate], given);
      case "TSAsExpression":
      case "TSSatisfiesExpression":
      case "TSNonNullExpression":
      case "TSTypeAssertion":
      case "TSInstantiationExpression":
      case "ParenthesizedExpression":
        return this.#nonData(node.expression, given);
      default:
        return node;
    }
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

  #source(node: t.Node): string {
    const text = this.text.slice(node.start ?? 0, node.end ?? 0).replace(/\s+/gu, " ");
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
      if (current.type === "TSAsExpression" || current.type === "TSSatisfiesExpression" || current.type === "TSNonNullExpression"
        || current.type === "TSTypeAssertion" || current.type === "ParenthesizedExpression") {
        current = current.expression;
      } else if (current.type === "Identifier" && this.#consts.has(current.name)) {
        current = this.#consts.get(current.name)!;
      } else {
        return current;
      }
    }
    return current;
  }

  /** The config objects `node` holds, in order, as ESLint flattens a config array. */
  #elements(node: t.Expression, scope: FlatConfigScope, depth: number): void {
    this.#step();
    const value = this.#unwrap(node, depth);
    if (depth > MAX_DEPTH) throw new LintConfigFormatError("it nests deeper than Rerail reads");
    if (value.type === "ArrayExpression") {
      for (const element of value.elements) {
        if (element === null) continue;
        this.#elements(element.type === "SpreadElement" ? element.argument : element, scope, depth + 1);
      }
    } else if (value.type === "ObjectExpression") {
      this.#object(value, scope, depth + 1);
    } else if (value.type === "CallExpression" && this.#isConfigHelper(value.callee)) {
      for (const argument of value.arguments) {
        if (argument.type === "SpreadElement") this.#elements(argument.argument, scope, depth + 1);
        else if (isExpression(argument)) this.#elements(argument, scope, depth + 1);
      }
    } else {
      this.#notRead(node, depth);
    }
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
   * What Rerail does not read in place of `node`: a value from an imported module is named as not read (see
   * `#unreadable`); anything of the file's own that Rerail cannot follow makes the file not statically readable.
   */
  #notRead(node: t.Expression, depth: number, own?: ReadonlyMap<string, Entry>): void {
    if (!this.#fromImport(node, depth)) throw new LintConfigFormatError(`Rerail cannot read ${this.#source(node)} as data`);
    this.#unreadable(this.#source(node), own);
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
    const callee = call.callee;
    if (call.type === "CallExpression" && isRequire(call) && this.#isGlobal("require")) return "global";
    if (call.type === "NewExpression" && callee.type === "Identifier" && callee.name === "URL" && this.#isGlobal("URL")) return "global";
    if (call.type === "CallExpression" && callee.type === "MemberExpression" && !callee.computed && callee.property.type === "Identifier") {
      const object = callee.object;
      if (object.type === "Identifier" && object.name === "process" && this.#isGlobal("process") && callee.property.name === "cwd") {
        return call.arguments.length === 0 ? "global" : undefined;
      }
      if (object.type === "MetaProperty" && object.meta.name === "import" && callee.property.name === "resolve") return "global";
    }
    let root: t.Node = callee;
    while (root.type === "MemberExpression" || root.type === "OptionalMemberExpression") root = root.object;
    if (root.type !== "Identifier") return undefined;
    const from = this.#imports.get(root.name)?.module;
    return from !== undefined && !isPathModule(from) && !CODE_MODULES.has(from.replace(/^node:/u, "")) ? "package" : undefined;
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
   * A config object's properties by name; undefined where a spread, a computed key or a method (a getter could return
   * anything) hides what they are.
   */
  #properties(object: t.ObjectExpression, depth: number): Map<string, t.Expression> | undefined {
    this.#step();
    const properties = new Map<string, t.Expression>();
    for (const property of object.properties) {
      if (property.type === "SpreadElement") {
        const spread = this.#unwrap(property.argument, depth);
        if (spread.type !== "ObjectExpression") {
          // A spread imported config hides what the object holds; one of the file's own Rerail cannot follow is code.
          if (!this.#fromImport(property.argument, depth)) throw new LintConfigFormatError(`Rerail cannot read ${this.#source(property.argument)} as data`);
          return undefined;
        }
        const inner = this.#properties(spread, depth + 1);
        if (inner === undefined) return undefined;
        for (const [name, value] of inner) properties.set(name, value);
        continue;
      }
      if (property.type !== "ObjectProperty" || !isExpression(property.value)) throw new LintConfigFormatError(`Rerail cannot read ${this.#source(property)} as data`);
      const name = propertyName(property) ?? this.#computedKey(property, depth);
      if (name === undefined) throw new LintConfigFormatError(`Rerail cannot read ${this.#source(property)} as data`);
      properties.set(name, property.value);
    }
    return properties;
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
    if (extended !== undefined) this.#elements(extended, scope, depth + 1);
    const rules = properties.get("rules");
    if (rules !== undefined) this.#rules(rules, scope, depth + 1);
  }

  #scope(properties: ReadonlyMap<string, t.Expression>, inherited: FlatConfigScope, depth: number): FlatConfigScope {
    const files = properties.get("files");
    const limited = files !== undefined || properties.has("ignores") || properties.has("basePath");
    if (!limited) return inherited;
    const text = files === undefined ? "some files" : this.#source(files);
    // Within an `extends` of a limited object the files are the intersection; Rerail maps neither.
    if (inherited.kind !== "all" || files === undefined || properties.has("ignores") || properties.has("basePath")) return { kind: "some", files: text };
    const patterns = this.#resolve(files, depth);
    const folders = patterns.ok && Array.isArray(patterns.value) && patterns.value.length > 0
      ? patterns.value.map(pattern => (typeof pattern === "string" ? folderOfGlob(pattern) : undefined))
      : [];
    return folders.length > 0 && folders.every(folder => folder !== undefined)
      ? { kind: "folders", folders: folders as string[], files: text }
      : { kind: "some", files: text };
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
      this.#notRead(node, depth, own);
      return;
    }
    for (const property of rules.properties) {
      if (property.type === "SpreadElement") {
        this.#gatherRules(property.argument, scope, depth + 1, own);
        continue;
      }
      const key = property.type === "ObjectProperty" ? propertyName(property) ?? this.#computedKey(property, depth) : undefined;
      if (key === undefined) throw new LintConfigFormatError(`Rerail cannot read ${this.#source(property)} as data`);
      if (property.type !== "ObjectProperty" || !isExpression(property.value)) {
        own.delete(key);
        continue;
      }
      const value = this.#resolve(property.value, depth + 1);
      own.set(key, { key, scope, value: value.ok ? { ok: true, value: value.value } : { ok: false, why: value.why } });
    }
  }

  #computedKey(property: t.ObjectProperty, depth: number): string | undefined {
    if (!property.computed || !isExpression(property.key)) return undefined;
    const key = this.#resolve(property.key, depth + 1);
    return key.ok && typeof key.value === "string" ? key.value : undefined;
  }

  /** `node` as JSON where it is literal, through same-file `const` bindings and spreads; why not otherwise. */
  #resolve(node: t.Expression, depth: number): Resolved {
    this.#step();
    if (depth > MAX_DEPTH) return { ok: false, why: "it nests too deep" };
    const value = this.#unwrap(node, depth);
    const unread = (): Resolved => ({ ok: false, why: `it is ${this.#source(value)}, which Rerail does not run` });
    switch (value.type) {
      case "StringLiteral":
      case "NumericLiteral":
      case "BooleanLiteral":
        return { ok: true, value: value.value };
      case "NullLiteral":
        return { ok: true, value: null };
      case "TemplateLiteral":
        return value.expressions.length === 0 && value.quasis.length === 1 && value.quasis[0]!.value.cooked != null
          ? { ok: true, value: value.quasis[0]!.value.cooked }
          : unread();
      case "UnaryExpression": {
        if ((value.operator !== "-" && value.operator !== "+") || value.argument.type !== "NumericLiteral") return unread();
        return { ok: true, value: value.operator === "-" ? -value.argument.value : value.argument.value };
      }
      case "ArrayExpression": {
        const items: Json[] = [];
        for (const element of value.elements) {
          if (element === null) return unread();
          const inner = this.#resolve(element.type === "SpreadElement" ? element.argument : element, depth + 1);
          if (!inner.ok) return inner;
          if (element.type === "SpreadElement") {
            if (!Array.isArray(inner.value)) return unread();
            this.#built(inner.value.length);
            for (const item of inner.value) items.push(item);
          } else {
            this.#built(1);
            items.push(inner.value);
          }
        }
        return { ok: true, value: items };
      }
      case "ObjectExpression": {
        const object: { [key: string]: Json } = {};
        for (const property of value.properties) {
          if (property.type === "SpreadElement") {
            const inner = this.#resolve(property.argument, depth + 1);
            if (!inner.ok) return inner;
            if (inner.value === null || typeof inner.value !== "object" || Array.isArray(inner.value)) return unread();
            this.#built(Object.keys(inner.value).length);
            Object.assign(object, inner.value);
            continue;
          }
          const key = property.type === "ObjectProperty" ? propertyName(property) ?? this.#computedKey(property, depth) : undefined;
          if (key === undefined || property.type !== "ObjectProperty" || !isExpression(property.value)) return unread();
          const inner = this.#resolve(property.value, depth + 1);
          if (!inner.ok) return inner;
          this.#built(1);
          Object.defineProperty(object, key, { value: inner.value, enumerable: true, writable: true, configurable: true });
        }
        return { ok: true, value: object };
      }
      default:
        return unread();
    }
  }
}

function isExpression(node: t.Node | null | undefined): node is t.Expression {
  return node != null && !node.type.endsWith("Declaration") && node.type !== "ArgumentPlaceholder" && node.type !== "JSXNamespacedName";
}

/** A property's name where it is written out: `rules`, `"no-console"`, `1`; undefined for a computed key. */
function propertyName(property: t.ObjectProperty | t.ObjectMethod): string | undefined {
  if (property.computed) return undefined;
  const key = property.key;
  if (key.type === "Identifier") return key.name;
  if (key.type === "StringLiteral") return key.value;
  if (key.type === "NumericLiteral") return String(key.value);
  return undefined;
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
