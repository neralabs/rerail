/**
 * A flat config file as parsed: its text, and the names it binds. The file's imports and top-level `const`s are read
 * before anything else, so that both the check that the file is data (`eslint-flat-config-code-check.ts`) and the
 * reading of its config can tell what a name is: a `const` of the file, an import from a package or from a path, or a
 * global; and which calls go to a package the file imports, a config helper, or a global Rerail trusts.
 */
import type * as t from "@babel/types";

const MAX_SOURCE_TEXT = 48;
const DEFINE_CONFIG_MODULES = new Set(["eslint/config", "@eslint/config-helpers"]);
const TYPESCRIPT_ESLINT_MODULES = new Set(["typescript-eslint"]);
/** How deep a value may nest or a binding chain may go before it is not read. */
export const MAX_DEPTH = 64;
/** Node modules that run code, or reach the module system or the process, and so can change anything when called. */
const CODE_MODULES = new Set(["vm", "module", "inspector", "repl", "process", "worker_threads", "child_process", "v8", "wasi"]);

/** Whether `module` is named by a path (which may be the config itself or a file of the repository), not a package. */
export const isPathModule = (module: string): boolean => /^(?:\.|\/|[A-Za-z]:|file:)/u.test(module);

/** TypeScript's wrappers around an expression (`x as T`, `x satisfies T`, `x!`, `<T>x`): erased before the file runs. */
type TypeWrapper = t.TSAsExpression | t.TSSatisfiesExpression | t.TSNonNullExpression | t.TSTypeAssertion;
const TYPE_WRAPPERS = new Set(["TSAsExpression", "TSSatisfiesExpression", "TSNonNullExpression", "TSTypeAssertion"]);
const isTypeWrapper = (node: t.Node): node is TypeWrapper => TYPE_WRAPPERS.has(node.type);

/** The name of a member or object key, or undefined when it is computed from anything but a literal. */
export function keyName(key: t.Node, computed: boolean): string | undefined {
  if (!computed && key.type === "Identifier") return key.name;
  if (key.type === "StringLiteral") return key.value;
  if (key.type === "NumericLiteral") return String(key.value);
  return undefined;
}

/** An import of the file: the module it is from and the name imported (`default`, `*` or the export). */
interface Import {
  readonly module: string;
  readonly imported: string;
}

export class FlatConfigFile {
  readonly #consts = new Map<string, t.Expression>();
  /** Every top-level const of the file, exported ones included: none may be handed to a call (`eslint-flat-config-code-check.ts`). */
  readonly #locals = new Set<string>();
  /** Local name → what it imports. */
  readonly #imports = new Map<string, Import>();

  constructor(readonly text: string, readonly program: t.Program) {
    this.#bindings();
    this.#forgetExportedConsts();
  }

  /** The file's imports and top-level consts. */
  #bindings(): void {
    for (const statement of this.program.body) {
      if (statement.type === "ImportDeclaration") this.#import(statement);
      else if (statement.type === "VariableDeclaration") this.#declare(statement, false);
      else if (statement.type === "ExportNamedDeclaration" && statement.declaration?.type === "VariableDeclaration") this.#declare(statement.declaration, true);
    }
  }

  /** A const another module can import is not read: that module may change it. */
  #forgetExportedConsts(): void {
    for (const statement of this.program.body) {
      if (statement.type !== "ExportNamedDeclaration") continue;
      for (const specifier of statement.specifiers) {
        if (specifier.type === "ExportSpecifier" && specifier.local.type === "Identifier") this.#consts.delete(specifier.local.name);
      }
    }
  }

  /** What the file imports as `name`; undefined where it imports no such name. */
  imported(name: string): Import | undefined {
    return this.#imports.get(name);
  }

  /** Whether `name` is a top-level const of the file. */
  isLocal(name: string): boolean {
    return this.#locals.has(name);
  }

  #import(statement: t.ImportDeclaration): void {
    // A type-only import is erased before the file runs: its names are the globals of those names there.
    if (isTypeOnly(statement.importKind)) return;
    for (const specifier of statement.specifiers) {
      if (specifier.type === "ImportSpecifier" && isTypeOnly(specifier.importKind)) continue;
      this.#imports.set(specifier.local.name, { module: statement.source.value, imported: importedName(specifier) });
    }
  }

  #declare(declaration: t.VariableDeclaration, exported: boolean): void {
    // Only a const keeps its value; a `let` or `var` may be reassigned anywhere, so it is not read.
    if (declaration.kind !== "const") return;
    for (const declarator of declaration.declarations) this.#declarator(declarator, exported);
  }

  #declarator(declarator: t.VariableDeclarator, exported: boolean): void {
    if (declarator.id.type !== "Identifier" || declarator.init == null) return;
    if (declarator.init.type === "CallExpression" && isRequire(declarator.init)) {
      // A required module is an import: Rerail takes it to behave as documented, as it does `import`s.
      this.#imports.set(declarator.id.name, { module: requiredModule(declarator.init), imported: "default" });
      return;
    }
    this.#locals.add(declarator.id.name);
    // An exported const is not read: a module that imports it may change it.
    if (!exported) this.#consts.set(declarator.id.name, declarator.init);
  }

  /** The value of `module.exports = <value>` where `module` is CommonJS's own, not a binding of the file. */
  moduleExports(expression: t.Expression): t.Expression | undefined {
    return this.#locals.has("module") || this.#imports.has("module") ? undefined : moduleExports(expression);
  }

  /** How `node` is written in the file, on one line and cut short. The parser gives every node its place in the text. */
  source(node: t.Node): string {
    const text = this.text.slice(node.start!, node.end!).replace(/\s+/gu, " ");
    return text.length > MAX_SOURCE_TEXT ? `${text.slice(0, MAX_SOURCE_TEXT - 1)}…` : text;
  }

  /** `node` without TypeScript's type wrappers, and a same-file `const` it names replaced by its value. */
  unwrap(node: t.Expression, depth: number): t.Expression {
    let current = node;
    for (let step = depth; step < MAX_DEPTH; step += 1) {
      const inner = isTypeWrapper(current) ? current.expression : current.type === "Identifier" ? this.#consts.get(current.name) : undefined;
      if (inner === undefined) return current;
      current = inner;
    }
    return current;
  }

  /**
   * Whether `call` calls into a package the file imports or requires, through any member path; or is
   * `require("<module>")`, `new URL(…)`, `process.cwd()` or `import.meta.resolve(…)` (globals nothing in the file has
   * replaced). A module the file names by path (`./x`, `/x`) may be the config itself, and a Node module that runs code
   * or reaches the module system (`node:vm`, `node:module`, ...) can change anything: calls into them are code.
   */
  trustedCall(call: t.CallExpression | t.OptionalCallExpression | t.NewExpression): "global" | "package" | undefined {
    if (this.#isTrustedGlobalCall(call)) return "global";
    const root = memberRoot(call.callee);
    if (root.type !== "Identifier") return undefined;
    return isPackageModule(this.#imports.get(root.name)?.module) ? "package" : undefined;
  }

  #isTrustedGlobalCall(call: t.CallExpression | t.OptionalCallExpression | t.NewExpression): boolean {
    return this.#isGlobalRequire(call) || this.#isGlobalNewUrl(call) || this.#isGlobalProcessCwd(call) || isImportMetaResolve(call);
  }

  #isGlobalRequire(call: t.CallExpression | t.OptionalCallExpression | t.NewExpression): boolean {
    return call.type === "CallExpression" && isRequire(call) && this.isGlobal("require");
  }

  #isGlobalNewUrl(call: t.CallExpression | t.OptionalCallExpression | t.NewExpression): boolean {
    return call.type === "NewExpression" && call.callee.type === "Identifier" && call.callee.name === "URL" && this.isGlobal("URL");
  }

  #isGlobalProcessCwd(call: t.CallExpression | t.OptionalCallExpression | t.NewExpression): boolean {
    const method = plainMethodCall(call);
    return method !== undefined && this.isGlobalProcess(method.object) && method.name === "cwd" && call.arguments.length === 0;
  }

  /** The `process` of the global scope: the file neither declares nor imports one. */
  isGlobalProcess(node: t.Node): boolean {
    return node.type === "Identifier" && node.name === "process" && this.isGlobal("process");
  }

  /** Whether `name` is the global of that name: the file neither declares nor imports it. */
  isGlobal(name: string): boolean {
    return !this.#locals.has(name) && !this.#imports.has(name);
  }

  /** `defineConfig` imported from ESLint, or typescript-eslint's `config`: both concatenate their arguments. */
  isConfigHelper(callee: t.Node): boolean {
    if (callee.type === "Identifier") return this.#isConfigHelperNamed(callee, "defineConfig");
    return callee.type === "MemberExpression" && !callee.computed && callee.property.type === "Identifier" && callee.property.name === "config"
      && this.#isTypescriptEslint(callee.object);
  }

  /** Whether `object` is typescript-eslint, through its default or namespace import. */
  #isTypescriptEslint(object: t.Node): boolean {
    const from = object.type === "Identifier" ? this.#imports.get(object.name) : undefined;
    return from !== undefined && TYPESCRIPT_ESLINT_MODULES.has(from.module) && (from.imported === "default" || from.imported === "*");
  }

  /** Whether `callee` is ESLint's config helper `name`, imported from `eslint/config` or `@eslint/config-helpers`. */
  #isConfigHelperNamed(callee: t.Identifier, name: string): boolean {
    const from = this.#imports.get(callee.name);
    return from !== undefined && DEFINE_CONFIG_MODULES.has(from.module) && from.imported === name;
  }

  /** A call of ESLint's `globalIgnores`, which makes a config of the globs it is given. */
  isGlobalIgnoresCall(node: t.Node): node is t.CallExpression {
    return node.type === "CallExpression" && node.callee.type === "Identifier" && this.#isConfigHelperNamed(node.callee, "globalIgnores");
  }
}

/** Whether an import or one of its names imports types alone. */
const isTypeOnly = (kind: t.ImportDeclaration["importKind"] | t.ImportSpecifier["importKind"]): boolean => kind === "type" || kind === "typeof";

/** The name an import specifier imports: the export's, `default` or `*`. */
function importedName(specifier: t.ImportDeclaration["specifiers"][number]): string {
  return specifier.type === "ImportSpecifier"
    ? (specifier.imported.type === "Identifier" ? specifier.imported.name : specifier.imported.value)
    : specifier.type === "ImportDefaultSpecifier" ? "default" : "*";
}

/** What a member path `a.b.c` hangs off: `a`. */
export function memberRoot(node: t.Node): t.Node {
  let root = node;
  while (root.type === "MemberExpression" || root.type === "OptionalMemberExpression") root = root.object;
  return root;
}

export const isCodeModule = (module: string | undefined): boolean => module !== undefined && CODE_MODULES.has(module.replace(/^node:/u, ""));

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

/** Whether `export default` exports an expression rather than a declaration (`function`, `class`). */
export function isExpression(declared: t.ExportDefaultDeclaration["declaration"]): declared is t.Expression {
  return !declared.type.endsWith("Declaration");
}

/**
 * A property written out in an object of a file read as data, as its key and value. The code check
 * (`eslint-flat-config-code-check.ts`) lets no method through, nor any key but a name or a literal; and a property of
 * an object literal holds an expression.
 */
export function dataProperty(property: t.ObjectProperty | t.ObjectMethod): [string, t.Expression] {
  return [keyName(property.key, property.computed)!, (property as t.ObjectProperty).value as t.Expression];
}

/** The value of `module.exports = <value>`, or undefined for any other expression. */
function moduleExports(expression: t.Expression): t.Expression | undefined {
  if (expression.type !== "AssignmentExpression" || expression.operator !== "=") return undefined;
  return isModuleExports(expression.left) ? expression.right : undefined;
}

/** Whether an assignment's `target` is `module.exports`. */
function isModuleExports(target: t.AssignmentExpression["left"]): boolean {
  return target.type === "MemberExpression" && !target.computed && target.object.type === "Identifier"
    && target.object.name === "module" && target.property.type === "Identifier" && target.property.name === "exports";
}

export function isRequire(call: t.CallExpression): boolean {
  return call.callee.type === "Identifier" && call.callee.name === "require" && call.arguments.length === 1 && call.arguments[0]!.type === "StringLiteral";
}

export function requiredModule(call: t.CallExpression): string {
  return (call.arguments[0] as t.StringLiteral).value;
}
