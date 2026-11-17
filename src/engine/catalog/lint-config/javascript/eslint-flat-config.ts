/**
 * ESLint's flat config (`eslint.config.{js,mjs,cjs,ts,mts,cts}`) read as data: the file is parsed, never run or
 * imported, and read only when it is data throughout (`eslint-flat-config-code-check.ts`). The rule entries of the
 * exported config array are read through same-file `const` bindings, array and object spreads, `defineConfig(...)`
 * (from `eslint/config` or `@eslint/config-helpers`), typescript-eslint's `config(...)` and
 * `extends`; `globalIgnores(...)` from the same modules is an element of `ignores` alone. Rerail takes those helpers,
 * and the modules the file imports, to do what their documentation says; it does not read them. A shared config it
 * recognises by the module the file imports and the member path it reads
 * (`js.configs.recommended`, `...tseslint.configs.strict`, `...react.configs.flat.recommended` in an object,
 * `...js.configs.recommended.rules` in `rules`: `eslint-presets.ts`), or by a string `extends` through the plugin the
 * object binds (`plugins: { js }, extends: ["js/recommended"]`), contributes its rule entries in place, marked with the
 * preset. A namespace `plugins` binds to a plugin it recognises names that plugin's rules (`plugins: { ts:
 * tseslint.plugin }` makes `ts/no-explicit-any` typescript-eslint's `no-explicit-any`); one bound to anything else names
 * a plugin Rerail cannot tell. Anything else the file takes from them (an unrecognised shared config, an imported value,
 * a call's result) is not evaluated: a rule entry whose value it decides is reported as unresolved, and a config element
 * it is is named in a note and in `unread`.
 */
import { createRequire } from "node:module";
import type * as BabelParser from "@babel/parser";
import type * as t from "@babel/types";
import { flatPlugin, flatPreset, pluginPreset, type FlatPlugin } from "./eslint-presets.js";
import type { EslintPresetEntry } from "../generated/eslint-presets.generated.js";
import { LintConfigFormatError } from "../formats/lint-config-formats.js";
import { refuseCode } from "./eslint-flat-config-code-check.js";
import { dataProperty, FlatConfigFile, isExpression, keyName, MAX_DEPTH, memberRoot } from "./eslint-flat-config-file.js";
import { filesScope, narrowScope, presetScope } from "./eslint-flat-config-scope.js";
import { FlatConfigValues, ReadBudget, type Resolved } from "./eslint-flat-config-values.js";

/** The parser, loaded on the first flat config: every `rerail` command imports this module, few parse one. */
let babel: typeof BabelParser | undefined;
const loadParser = (): typeof BabelParser => (babel ??= createRequire(import.meta.url)("@babel/parser") as typeof BabelParser);

/** Where a flat config's rule entry applies. */
export type FlatConfigScope =
  /** Every file: the config object has no `files`, `ignores` or `basePath`. */
  | { readonly kind: "all" }
  /** Exactly the files under these folders (relative to the repository root), from `files: ["<folder>/**"]`. */
  | { readonly kind: "folders"; readonly folders: readonly string[]; readonly files: string; readonly patterns?: FilePatterns }
  /** Some files Rerail does not map onto folders; `files` is how the config names them, `patterns` the globs where Rerail reads them. */
  | { readonly kind: "some"; readonly files: string; readonly patterns?: FilePatterns };

/**
 * The files an entry applies to as globs from the repository root: those that match a glob of every group of `files`
 * (an element's own, and the elements' it is extended in), less those a glob of `ignores` matches.
 */
export interface FilePatterns {
  readonly files: readonly (readonly string[])[];
  readonly ignores: readonly string[];
}

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
  /**
   * Each namespace the config's `plugins` bind, with the namespace the plugin's own rules are named under
   * (`ts` → `@typescript-eslint`); undefined where Rerail cannot tell which plugin it is.
   */
  readonly namespaces: ReadonlyMap<string, string | undefined>;
  /** A `plugins` value Rerail does not read, which may bind any namespace, as the file names it. */
  readonly pluginsUnread?: string;
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
  return new FlatConfigReader(new FlatConfigFile(text, program)).read();
}

class FlatConfigReader {
  readonly #file: FlatConfigFile;
  readonly #budget = new ReadBudget();
  readonly #values: FlatConfigValues;
  readonly #entries: Entry[] = [];
  readonly #unread: string[] = [];
  readonly #notes: string[] = [];
  readonly #namespaces = new Map<string, string | undefined>();
  #pluginsUnread: string | undefined;

  constructor(file: FlatConfigFile) {
    this.#file = file;
    this.#values = new FlatConfigValues(file, this.#budget);
  }

  read(): FlatConfigRead {
    const exported = this.#exported();
    refuseCode(this.#file, exported);
    this.#elements(exported, { kind: "all" }, 0);
    if (this.#unread.length > 0) {
      this.#notes.push(`the rules of ${this.#unread.join(", ")} are not read: Rerail does not run code to find them.`);
    }
    const pluginsUnread = this.#pluginsUnread;
    return { entries: this.#entries, unread: this.#unread, notes: this.#notes, namespaces: this.#namespaces, ...(pluginsUnread === undefined ? {} : { pluginsUnread }) };
  }

  /** What `statement` exports as the config, if it exports one: `export default`, `export =` or `module.exports = …`. */
  #exportOf(statement: t.Statement): t.Expression | null | undefined {
    if (statement.type === "ExportDefaultDeclaration") return isExpression(statement.declaration) ? statement.declaration : null;
    if (statement.type === "TSExportAssignment") return statement.expression;
    return statement.type === "ExpressionStatement" ? this.#file.moduleExports(statement.expression) : undefined;
  }

  /** The config the file exports: the last `export default` or `module.exports = …`; throws where there is none, or both kinds. */
  #exported(): t.Expression {
    // An `export default` of a declaration exports no config Rerail reads (`null`); a later export replaces it.
    const exports = this.#file.program.body.map(statement => this.#exportOf(statement)).filter(found => found !== undefined);
    const exported = exports.at(-1);
    if (exported == null) throw new LintConfigFormatError("it exports no config Rerail can find (export default or module.exports)");
    const esm = this.#file.program.body.some(statement => statement.type === "ExportDefaultDeclaration" || statement.type === "TSExportAssignment");
    const cjs = this.#file.program.body.some(statement => statement.type === "ExpressionStatement" && this.#file.moduleExports(statement.expression) !== undefined);
    if (esm && cjs) throw new LintConfigFormatError("it exports its config both as a module and through module.exports");
    return exported;
  }

  /** The config objects `node` holds, in order, as ESLint flattens a config array. */
  #elements(node: t.Expression, scope: FlatConfigScope, depth: number): void {
    this.#budget.step();
    const value = this.#file.unwrap(node, depth);
    if (depth > MAX_DEPTH) throw new LintConfigFormatError("it nests deeper than Rerail reads");
    const nested = this.#nestedElements(value);
    if (nested !== undefined) for (const element of nested) this.#elements(element, scope, depth + 1);
    else if (value.type === "ObjectExpression") this.#object(value, scope, depth + 1);
    else if (!this.#isGlobalIgnores(value, depth)) this.#notRead(node, scope, depth);
  }

  /**
   * Whether `value` is a `globalIgnores(...)` that gives a config: a non-empty list of globs, with a name or none. Its
   * config holds `ignores` alone, which sets no rule (`#object`); one ESLint refuses is not read.
   */
  #isGlobalIgnores(value: t.Expression, depth: number): boolean {
    if (!this.#file.isGlobalIgnoresCall(value)) return false;
    return givesIgnoresConfig(value.arguments.map(argument => (argument.type === "SpreadElement" ? undefined : this.#values.resolve(argument as t.Expression, depth + 1))));
  }

  /** The elements an array or a config helper's call holds, spreads taken apart; undefined for any other value. */
  #nestedElements(value: t.Expression): t.Expression[] | undefined {
    const items = value.type === "ArrayExpression" ? value.elements
      : value.type === "CallExpression" && this.#file.isConfigHelper(value.callee) ? value.arguments
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
    this.#budget.step();
    let root: t.Node = this.#file.unwrap(node, depth);
    if (root.type === "ConditionalExpression") return this.#choiceFromImport(root, depth);
    if ((root.type === "CallExpression" || root.type === "OptionalCallExpression") && this.#file.trustedCall(root) === "package") root = root.callee;
    root = memberRoot(root);
    return root.type === "Identifier" && this.#file.imported(root.name) !== undefined;
  }

  /** Whether a choice is between values from imported modules, or nothing, and not between nothing and nothing. */
  #choiceFromImport(choice: t.ConditionalExpression, depth: number): boolean {
    const branches = [choice.consequent, choice.alternate].map(branch => this.#file.unwrap(branch, depth + 1));
    return branches.every(branch => isEmptyLiteral(branch) || this.#fromImport(branch, depth + 1)) && !branches.every(isEmptyLiteral);
  }

  /**
   * What Rerail does not read in place of `node`: a recognised shared config contributes its entries in place
   * (`#addPreset`); any other value from an imported module is named as not read (see `#unreadable`); anything of the
   * file's own that Rerail cannot follow makes the file not statically readable.
   */
  #notRead(node: t.Expression, scope: FlatConfigScope, depth: number, own?: Map<string, Entry>): void {
    if (!this.#fromImport(node, depth)) throw new LintConfigFormatError(`Rerail cannot read ${this.#file.source(node)} as data`);
    const preset = this.#presetOf(node, depth);
    if (preset === undefined) this.#unreadable(this.#file.source(node), own);
    else if (own === undefined) this.#addPreset(preset, scope);
    else for (const entry of preset.entries) this.#addPresetRules(preset.text, entry.rules, scope, own);
  }

  /**
   * The binding a member path (`js.configs.recommended`) hangs off, with the path's keys. A file read as data names its
   * members by literal keys only (`eslint-flat-config-code-check.ts`), and `super` does not parse outside a method,
   * which is code.
   */
  #memberPath(node: t.Expression, depth: number): { readonly root: t.Expression; readonly path: string[] } {
    const path: string[] = [];
    let current = this.#file.unwrap(node, depth);
    while (current.type === "MemberExpression" || current.type === "OptionalMemberExpression") {
      path.unshift(keyName(current.property, current.computed)!);
      current = this.#file.unwrap(current.object as t.Expression, depth);
    }
    return { root: current, path };
  }

  /**
   * The recognised shared config `node` is, with `member` after it: a member path of a binding the file imports that
   * names one of `eslint-presets.ts`'s presets.
   */
  #presetOf(node: t.Expression, depth: number, member: readonly string[] = []): Preset | undefined {
    const { root, path } = this.#memberPath(node, depth);
    const from = root.type === "Identifier" ? this.#file.imported(root.name) : undefined;
    const entries = from === undefined ? undefined : flatPreset(from.module, from.imported, [...path, ...member]);
    return entries === undefined ? undefined : { text: this.#file.source(node), entries };
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
    this.#budget.built(Object.keys(rules).length);
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
   * A config object's properties by name; undefined where a spread imported config Rerail does not recognise hides what
   * they are. A spread recognised shared config object gives its `files`, `ignores` and `rules`.
   */
  #properties(object: t.ObjectExpression, depth: number): Map<string, Property> | undefined {
    this.#budget.step();
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
    const spread = this.#file.unwrap(argument, depth);
    if (spread.type === "ObjectExpression") return this.#properties(spread, depth + 1);
    // A spread imported config hides what the object holds; one of the file's own Rerail cannot follow is code.
    if (!this.#fromImport(argument, depth)) throw new LintConfigFormatError(`Rerail cannot read ${this.#file.source(argument)} as data`);
    // Only a config object has a `rules` member preset; spreading an array adds no config key.
    const preset = this.#presetOf(argument, depth, ["rules"]) === undefined ? undefined : this.#presetOf(argument, depth);
    const entry = preset?.entries[0];
    if (entry === undefined) return undefined;
    return new Map(Object.entries(entry).map(([name, json]) => [name, new PresetValue(preset!.text, json)]));
  }

  #object(object: t.ObjectExpression, inherited: FlatConfigScope, depth: number): void {
    const properties = this.#properties(object, depth);
    if (properties === undefined) {
      this.#unreadable(this.#file.source(object));
      return;
    }
    const keys = [...properties.keys()].filter(key => key !== "name");
    // An object with nothing but `ignores` ignores those files everywhere; it sets no rule.
    if (keys.length === 1 && keys[0] === "ignores") return;
    const scope = this.#scope(properties, inherited, depth);
    const plugins = this.#bindPlugins(properties.get("plugins"), depth);
    const extended = properties.get("extends");
    if (extended !== undefined && !(extended instanceof PresetValue)) this.#extends(extended, plugins, scope, depth + 1);
    this.#objectRules(properties.get("rules"), scope, depth);
  }

  /**
   * A config object's `plugins`, each namespace bound to the recognised plugin it names, or undefined where Rerail
   * cannot tell which plugin it is. A namespace one object binds to one plugin and another to another is undefined too.
   */
  #bindPlugins(plugins: Property | undefined, depth: number): ReadonlyMap<string, FlatPlugin | undefined> {
    const bound = new Map<string, FlatPlugin | undefined>();
    // A recognised shared config spread into the object gives no `plugins` (`#spreadProperties`).
    if (plugins === undefined || plugins instanceof PresetValue) return bound;
    const object = this.#file.unwrap(plugins, depth);
    if (object.type !== "ObjectExpression") {
      this.#pluginsUnread ??= this.#file.source(plugins);
      return bound;
    }
    for (const property of object.properties) {
      if (property.type === "SpreadElement") this.#pluginsUnread ??= this.#file.source(property);
      else this.#bindPlugin(bound, ...dataProperty(property), depth);
    }
    return bound;
  }

  /** Binds `namespace` to the plugin `node` is, in `bound` and for the whole file. */
  #bindPlugin(bound: Map<string, FlatPlugin | undefined>, namespace: string, node: t.Expression, depth: number): void {
    const plugin = this.#pluginOf(node, depth);
    bound.set(namespace, plugin);
    // A plugin of configs alone (`@eslint/js`) has no rules to name: its namespace names itself.
    const canonical = plugin === undefined ? undefined : plugin.namespace ?? namespace;
    const agrees = !this.#namespaces.has(namespace) || this.#namespaces.get(namespace) === canonical;
    this.#namespaces.set(namespace, agrees ? canonical : undefined);
  }

  /** The recognised plugin `node` is: a member path of a binding the file imports that names one of `eslint-presets.ts`'s plugins. */
  #pluginOf(node: t.Expression, depth: number): FlatPlugin | undefined {
    const { root, path } = this.#memberPath(node, depth);
    const from = root.type === "Identifier" ? this.#file.imported(root.name) : undefined;
    return from === undefined ? undefined : flatPlugin(from.module, from.imported, path);
  }

  /**
   * A config object's `extends`, applied before its own rules: config arrays and objects as elements, and a string
   * `"<namespace>/<config>"` as `defineConfig` reads it, a config of the plugin the object binds under that namespace.
   */
  #extends(extended: t.Expression, plugins: ReadonlyMap<string, FlatPlugin | undefined>, scope: FlatConfigScope, depth: number): void {
    const list = this.#file.unwrap(extended, depth);
    if (list.type !== "ArrayExpression") {
      this.#elements(extended, scope, depth);
      return;
    }
    for (const element of this.#nestedElements(list)!) {
      const value = this.#file.unwrap(element, depth);
      if (value.type === "StringLiteral") this.#extendsName(value.value, plugins, scope);
      else this.#elements(element, scope, depth + 1);
    }
  }

  /**
   * A string in `extends`: the config `defineConfig` finds in the plugin bound under its namespace, with the rule
   * names of the plugin's own namespace (`meta.namespace`) renamed to the one the object binds it under (@eslint/config-helpers
   * 0.7.0 `normalizePluginConfig`). One Rerail does not recognise is not read.
   */
  #extendsName(name: string, plugins: ReadonlyMap<string, FlatPlugin | undefined>, scope: FlatConfigScope): void {
    const { namespace, name: config } = pluginMember(name);
    const plugin = plugins.get(namespace);
    const entries = plugin === undefined ? undefined : pluginPreset(plugin, config);
    if (entries === undefined) {
      this.#unreadable(JSON.stringify(name));
      return;
    }
    const meta = plugin!.meta;
    const renamed = meta === undefined || meta === namespace ? entries : entries.map(entry => ({
      ...entry,
      rules: Object.fromEntries(Object.entries(entry.rules).map(([key, value]) => {
        const member = pluginMember(key);
        return [member.namespace === meta ? `${namespace}/${member.name}` : key, value];
      })),
    }));
    this.#addPreset({ text: JSON.stringify(name), entries: renamed }, scope);
  }

  /** A config object's `rules`: written in the file, or a recognised shared config object's, spread into it. */
  #objectRules(rules: Property | undefined, scope: FlatConfigScope, depth: number): void {
    if (rules instanceof PresetValue) this.#addPreset({ text: rules.preset, entries: [{ rules: rules.json as Record<string, unknown> }] }, scope);
    else if (rules !== undefined) this.#rules(rules, scope, depth + 1);
  }

  /** A config object's `files` (or `ignores`) as written (`text`) and as JSON, where Rerail can read it: `null` where it cannot. */
  #files(files: Property | undefined, depth: number): { readonly text: string; readonly patterns: unknown } {
    if (files === undefined) return { text: "some files", patterns: undefined };
    if (files instanceof PresetValue) return { text: JSON.stringify(files.json), patterns: files.json };
    const patterns = this.#values.resolve(files, depth);
    return { text: this.#file.source(files), patterns: patterns.ok ? patterns.value : null };
  }

  #scope(properties: ReadonlyMap<string, Property>, inherited: FlatConfigScope, depth: number): FlatConfigScope {
    const files = properties.get("files");
    const ignores = properties.get("ignores");
    if (files === undefined && ignores === undefined && !properties.has("basePath")) return inherited;
    const read = this.#files(files, depth);
    const ignored = this.#files(ignores, depth).patterns;
    // A `basePath` moves the patterns to a folder Rerail does not map.
    if (properties.has("basePath")) return { kind: "some", files: read.text };
    return inherited.kind === "all" ? filesScope(read.patterns, read.text, ignored) : narrowScope(inherited, read.patterns, ignored, read.text);
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
    this.#budget.step();
    if (depth > MAX_DEPTH) throw new LintConfigFormatError("it nests deeper than Rerail reads");
    const rules = this.#file.unwrap(node, depth);
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
    const value = this.#values.resolve(node, depth + 1);
    own.set(key, { key, scope, value: value.ok ? { ok: true, value: value.value } : { ok: false, why: value.why } });
  }
}

/**
 * A plugin member's id (`ts/recommended`, `@typescript-eslint/no-explicit-any`) as its namespace and name, as
 * `defineConfig` splits it (@eslint/config-helpers 0.7.0 `getPluginMember`): at the first `/`, or at the second after
 * an npm scope.
 */
function pluginMember(id: string): { readonly namespace: string; readonly name: string } {
  const first = id.indexOf("/");
  const second = id.startsWith("@") ? id.indexOf("/", first + 1) : -1;
  const split = second > 0 ? second : first;
  return split < 0 ? { namespace: "", name: id } : { namespace: id.slice(0, split), name: id.slice(split + 1) };
}

/** Whether `branch` is an empty array or object literal: nothing to the config it is in. */
function isEmptyLiteral(branch: t.Expression): boolean {
  return (branch.type === "ArrayExpression" && branch.elements.length === 0)
    || (branch.type === "ObjectExpression" && branch.properties.length === 0);
}

/**
 * Whether `globalIgnores` given `args` (each resolved; undefined for a spread) makes a config ESLint takes: a non-empty
 * list of globs, and a name or none (@eslint/config-helpers 0.7.0 `globalIgnores`, @eslint/config-array's schema). A
 * name Rerail cannot tell is a string is taken as one ESLint refuses.
 */
function givesIgnoresConfig(args: readonly (Resolved | undefined)[]): boolean {
  const [globs, name] = args.map(arg => (arg?.ok === true ? arg.value : undefined));
  const named = args.length < 2 || typeof name === "string";
  return named && Array.isArray(globs) && globs.length > 0 && globs.every(glob => typeof glob === "string");
}
