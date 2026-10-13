/**
 * ESLint's flat config (`eslint.config.{js,mjs,cjs,ts,mts,cts}`) read as data: the file is parsed, never run or
 * imported. What its exported config array says about rules is read wherever it is literal: rule entries of the config
 * objects in the array, through same-file `const` bindings, array and object spreads, `defineConfig(...)` (from
 * `eslint/config` or `@eslint/config-helpers`), typescript-eslint's `config(...)` and `extends`. Anything else (a shared
 * config such as `js.configs.recommended`, an imported value, a call) is not evaluated: a rule entry whose value it
 * decides is reported as unresolved, and a config element it is is named in a note.
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
  /** Consts code may reach (`#findEscapes`): their initializer is not their value, so they are not read. */
  readonly #escaped = new Set<string>();
  /** Config helper imports a reference may change (`#findEscapes`): their calls are not read. */
  readonly #untrustedHelpers = new Set<string>();
  /** Local name → the module it is imported from and the name imported (`default`, `*` or the export). */
  readonly #imports = new Map<string, { readonly module: string; readonly imported: string }>();
  readonly #entries: Entry[] = [];
  readonly #unread: string[] = [];
  readonly #notes: string[] = [];

  constructor(readonly text: string, readonly program: t.Program) {}

  read(): FlatConfigRead {
    let exported: t.Expression | undefined;
    for (const statement of this.program.body) {
      // An exported const is not read: another module may change it.
      if (statement.type === "ImportDeclaration") this.#import(statement);
      else if (statement.type === "VariableDeclaration") this.#declare(statement);
      else if (statement.type === "ExportDefaultDeclaration") exported = isExpression(statement.declaration) ? statement.declaration : undefined;
      else if (statement.type === "TSExportAssignment") exported = statement.expression;
      else if (statement.type === "ExpressionStatement") exported = moduleExports(statement.expression) ?? exported;
    }
    if (exported === undefined) throw new LintConfigFormatError("it exports no config Rerail can find (export default or module.exports)");
    this.#findEscapes();
    this.#elements(exported, { kind: "all" }, 0);
    if (this.#unread.length > 0) {
      this.#notes.push(`the rules of ${this.#unread.join(", ")} are not read: Rerail does not run code to find them.`);
    }
    return { entries: this.#entries, notes: this.#notes };
  }

  /**
   * Which `const`s Rerail may read at their initializer, by an allow-list rather than by guessing every way a value can
   * change: a const is read only when each reference to it is data the export is built from (an array element, an
   * object property's value or computed key, a spread, a config helper's argument, the exported value, or another
   * const's initializer, through TypeScript's type wrappers). Any other reference (a member access, a call argument, a
   * destructuring source, a loop, a return, a named export, a function body) lets code reach its objects, so it is not
   * read, nor is any const whose objects it holds. A config helper import is trusted only while every reference to it
   * calls it or reads through it.
   */
  #findEscapes(): void {
    const parents = parentsOf(this.program);
    const helperModule = (name: string): boolean => {
      const from = this.#imports.get(name);
      return from !== undefined && ((DEFINE_CONFIG_MODULES.has(from.module) && from.imported === "defineConfig")
        || (TYPESCRIPT_ESLINT_MODULES.has(from.module) && (from.imported === "default" || from.imported === "*")));
    };
    // Helpers first: a const passed to an untrusted helper escapes.
    for (const [node, { parent, key }] of parents) {
      if (node.type !== "Identifier" || !helperModule(node.name) || !isReference(node, parents)) continue;
      if (parent.type === "CallExpression" && key === "callee") continue;
      if ((parent.type === "MemberExpression" || parent.type === "OptionalMemberExpression") && key === "object") {
        let top: t.Node = parent;
        while (parents.get(top)?.key === "object" && /MemberExpression$/u.test(parents.get(top)!.parent.type)) top = parents.get(top)!.parent;
        const above = parents.get(top);
        const write = above !== undefined && ((above.parent.type === "AssignmentExpression" && above.key === "left")
          || above.parent.type === "UpdateExpression" || (above.parent.type === "UnaryExpression" && above.parent.operator === "delete"));
        const call = above !== undefined && /CallExpression$/u.test(above.parent.type) && above.key === "callee";
        if (!write && (!call || top === parent)) continue;
      }
      this.#untrustedHelpers.add(node.name);
    }
    const escaped = new Set<string>();
    const holds = new Map<string, Set<string>>();
    for (const [node] of parents) {
      if (node.type !== "Identifier" || !this.#consts.has(node.name) || !isReference(node, parents)) continue;
      const use = this.#dataUse(node, parents);
      if (use === "escape") escaped.add(node.name);
      else if (use !== "export") holds.set(use, (holds.get(use) ?? new Set()).add(node.name));
    }
    // A const that escapes lets code reach every const it holds.
    const queue = [...escaped];
    for (let name = queue.pop(); name !== undefined; name = queue.pop()) {
      if (this.#escaped.has(name)) continue;
      this.#escaped.add(name);
      queue.push(...holds.get(name) ?? []);
    }
  }

  /** Where a reference to a const ends up: the export, another top-level const's initializer, or anywhere else. */
  #dataUse(reference: t.Node, parents: ParentMap): "export" | "escape" | string {
    let node = reference;
    for (;;) {
      const up = parents.get(node);
      if (up === undefined) return "escape";
      const { parent, key } = up;
      switch (parent.type) {
        case "TSAsExpression":
        case "TSSatisfiesExpression":
        case "TSNonNullExpression":
        case "TSTypeAssertion":
        case "ParenthesizedExpression":
          if (key !== "expression") return "escape";
          node = parent;
          continue;
        case "ArrayExpression":
        case "SpreadElement":
          node = parent;
          continue;
        case "ObjectProperty": {
          const object = parents.get(parent);
          if ((key !== "value" && !(key === "key" && parent.computed)) || object?.parent.type !== "ObjectExpression") return "escape";
          node = object.parent;
          continue;
        }
        case "ObjectExpression":
          node = parent;
          continue;
        case "CallExpression":
          if (key !== "arguments" || !this.#isConfigHelper(parent.callee)) return "escape";
          node = parent;
          continue;
        case "VariableDeclarator": {
          const declaration = parents.get(parent);
          const atTop = declaration !== undefined && declaration.parent.type === "VariableDeclaration" && declaration.parent.kind === "const"
            && parents.get(declaration.parent)?.parent.type === "Program";
          return key === "init" && parent.id.type === "Identifier" && atTop ? parent.id.name : "escape";
        }
        case "ExportDefaultDeclaration":
        case "TSExportAssignment":
          return "export";
        case "AssignmentExpression":
          return key === "right" && moduleExports(parent) !== undefined ? "export" : "escape";
        default:
          return "escape";
      }
    }
  }

  #import(statement: t.ImportDeclaration): void {
    for (const specifier of statement.specifiers) {
      const imported = specifier.type === "ImportSpecifier"
        ? (specifier.imported.type === "Identifier" ? specifier.imported.name : specifier.imported.value)
        : specifier.type === "ImportDefaultSpecifier" ? "default" : "*";
      this.#imports.set(specifier.local.name, { module: statement.source.value, imported });
    }
  }

  #declare(declaration: t.VariableDeclaration): void {
    // Only a const keeps its value; a `let` or `var` may be reassigned anywhere, so it is not read.
    if (declaration.kind !== "const") return;
    for (const declarator of declaration.declarations) {
      if (declarator.id.type !== "Identifier" || declarator.init == null) continue;
      if (declarator.init.type === "CallExpression" && isRequire(declarator.init)) {
        this.#imports.set(declarator.id.name, { module: requiredModule(declarator.init), imported: "default" });
      } else {
        this.#consts.set(declarator.id.name, declarator.init);
      }
    }
  }

  #source(node: t.Node): string {
    const text = this.text.slice(node.start ?? 0, node.end ?? 0).replace(/\s+/gu, " ");
    return text.length > MAX_SOURCE_TEXT ? `${text.slice(0, MAX_SOURCE_TEXT - 1)}…` : text;
  }

  /** `node` without TypeScript's type wrappers, and a same-file `const` it names replaced by its value. */
  #unwrap(node: t.Expression, depth: number): t.Expression {
    let current = node;
    for (let step = depth; step < MAX_DEPTH; step += 1) {
      if (current.type === "TSAsExpression" || current.type === "TSSatisfiesExpression" || current.type === "TSNonNullExpression"
        || current.type === "TSTypeAssertion" || current.type === "ParenthesizedExpression") {
        current = current.expression;
      } else if (current.type === "Identifier" && this.#consts.has(current.name) && !this.#escaped.has(current.name)) {
        current = this.#consts.get(current.name)!;
      } else {
        return current;
      }
    }
    return current;
  }

  /** The config objects `node` holds, in order, as ESLint flattens a config array. */
  #elements(node: t.Expression, scope: FlatConfigScope, depth: number): void {
    const value = this.#unwrap(node, depth);
    if (depth > MAX_DEPTH) {
      this.#unreadable(this.#source(node));
      return;
    }
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
      this.#unreadable(this.#source(node));
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

  /** `defineConfig` imported from ESLint, or typescript-eslint's `config`: both concatenate their arguments. */
  #isConfigHelper(callee: t.Node): boolean {
    if (callee.type === "Identifier") {
      const from = this.#imports.get(callee.name);
      return from !== undefined && !this.#untrustedHelpers.has(callee.name) && DEFINE_CONFIG_MODULES.has(from.module) && from.imported === "defineConfig";
    }
    if (callee.type === "MemberExpression" && !callee.computed && callee.object.type === "Identifier" && callee.property.type === "Identifier") {
      const from = this.#imports.get(callee.object.name);
      return from !== undefined && !this.#untrustedHelpers.has(callee.object.name) && TYPESCRIPT_ESLINT_MODULES.has(from.module)
        && callee.property.name === "config" && (from.imported === "default" || from.imported === "*");
    }
    return false;
  }

  /**
   * A config object's properties by name; undefined where a spread, a computed key or a method (a getter could return
   * anything) hides what they are.
   */
  #properties(object: t.ObjectExpression, depth: number): Map<string, t.Expression> | undefined {
    const properties = new Map<string, t.Expression>();
    for (const property of object.properties) {
      if (property.type === "SpreadElement") {
        const spread = this.#unwrap(property.argument, depth);
        const inner = spread.type === "ObjectExpression" ? this.#properties(spread, depth + 1) : undefined;
        if (inner === undefined) return undefined;
        for (const [name, value] of inner) properties.set(name, value);
        continue;
      }
      if (property.type !== "ObjectProperty" || !isExpression(property.value)) return undefined;
      const name = propertyName(property);
      if (name === undefined) return undefined;
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
    this.#entries.push(...own.values());
  }

  #gatherRules(node: t.Expression, scope: FlatConfigScope, depth: number, own: Map<string, Entry>): void {
    const rules = this.#unwrap(node, depth);
    if (rules.type !== "ObjectExpression" || depth > MAX_DEPTH) {
      this.#unreadable(this.#source(node), own);
      return;
    }
    for (const property of rules.properties) {
      if (property.type === "SpreadElement") {
        this.#gatherRules(property.argument, scope, depth + 1, own);
        continue;
      }
      const key = property.type === "ObjectProperty" ? propertyName(property) ?? this.#computedKey(property, depth) : undefined;
      if (key === undefined) {
        this.#unreadable(`[${this.#source(property.key)}]`, own);
        continue;
      }
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
            items.push(...inner.value);
          } else {
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
            Object.assign(object, inner.value);
            continue;
          }
          const key = property.type === "ObjectProperty" ? propertyName(property) ?? this.#computedKey(property, depth) : undefined;
          if (key === undefined || property.type !== "ObjectProperty" || !isExpression(property.value)) return unread();
          const inner = this.#resolve(property.value, depth + 1);
          if (!inner.ok) return inner;
          Object.defineProperty(object, key, { value: inner.value, enumerable: true, writable: true, configurable: true });
        }
        return { ok: true, value: object };
      }
      default:
        return unread();
    }
  }
}

const NOT_CHILDREN = new Set(["loc", "extra", "leadingComments", "trailingComments", "innerComments", "comments", "tokens"]);
/** Children that are types, which hold no value: `Linter.Config` in an annotation is not a reference to `Linter`. */
const TYPE_CHILDREN = new Set(["typeAnnotation", "typeParameters", "returnType", "superTypeParameters", "implements", "typeArguments", "predicate"]);
const TYPE_DECLARATIONS = new Set(["TSTypeAliasDeclaration", "TSInterfaceDeclaration", "TSDeclareFunction", "TSModuleDeclaration", "ImportDeclaration"]);

const isNode = (value: unknown): value is t.Node =>
  typeof value === "object" && value !== null && "type" in value && typeof value.type === "string";

type ParentMap = ReadonlyMap<t.Node, { readonly parent: t.Node; readonly key: string }>;

/**
 * Every value node under `program` with its parent and the property holding it, without Babel's runtime. Types and
 * imports are left out: they hold no reference to a value.
 */
function parentsOf(program: t.Program): ParentMap {
  const parents = new Map<t.Node, { parent: t.Node; key: string }>();
  const seen = new Set<t.Node>([program]);
  const pending: t.Node[] = [program];
  for (let current = pending.pop(); current !== undefined; current = pending.pop()) {
    for (const [key, value] of Object.entries(current)) {
      if (NOT_CHILDREN.has(key) || TYPE_CHILDREN.has(key)) continue;
      for (const child of Array.isArray(value) ? value : [value]) {
        if (!isNode(child) || TYPE_DECLARATIONS.has(child.type)) continue;
        // Should a shorthand property's key and value be one node, it is the value: a reference.
        if (!parents.has(child) || key === "value") parents.set(child, { parent: current, key });
        if (!seen.has(child)) {
          seen.add(child);
          pending.push(child);
        }
      }
    }
  }
  return parents;
}

const PATTERNS = new Set(["ObjectPattern", "ArrayPattern", "RestElement", "AssignmentPattern"]);

/** Whether identifier `node` names a value it reads or writes, rather than a property name, a label or a new binding. */
function isReference(node: t.Node, parents: ParentMap): boolean {
  const up = parents.get(node);
  if (up === undefined) return false;
  const { parent, key } = up;
  if (/MemberExpression$/u.test(parent.type) && key === "property" && !("computed" in parent && parent.computed)) return false;
  if (key === "key" && "computed" in parent && !parent.computed) return false;
  if (key === "label" || (parent.type === "ExportSpecifier" && key === "exported")) return false;
  // A name in a declaration's pattern, a parameter, a function's or class's name, or a catch parameter is a new binding.
  let above: { readonly parent: t.Node; readonly key: string } | undefined = up;
  while (above !== undefined && (PATTERNS.has(above.parent.type)
    || (above.parent.type === "ObjectProperty" && PATTERNS.has(parents.get(above.parent)?.parent.type ?? "")))) {
    above = parents.get(above.parent);
  }
  if (above === undefined) return false;
  const binding = (above.parent.type === "VariableDeclarator" && above.key === "id") || above.key === "params" || above.key === "param"
    || (above.key === "id" && /Function|Class/u.test(above.parent.type));
  return !binding;
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
