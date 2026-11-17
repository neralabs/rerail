/**
 * The check that an ESLint flat config is data. A file is read only when it is data throughout: imports, top-level
 * `const`s, the export, and type-only declarations, built from literals, arrays, objects without methods, spreads, reads
 * of members, and calls that are given no local `const` (a config helper excepted); anything else (a function, a
 * statement that changes something, a namespace, a loop) is code, and the file is not read. Nothing in such a file
 * changes a `const` once set, so its config can be read through its bindings without running it.
 */
import type * as t from "@babel/types";
import { LintConfigFormatError } from "../formats/lint-config-formats.js";
import { isCodeModule, isExpression, isPathModule, isRequire, keyName, requiredModule, type FlatConfigFile } from "./eslint-flat-config-file.js";

/**
 * The only globals a data file may name outside its trusted calls (`FlatConfigFile.trustedCall`): any other name the file neither
 * declares nor imports (`require`, `module`, `process`, `globalThis`, `eval`, `arguments`, ...) can reach the
 * exported config or run code.
 */
const DATA_GLOBALS = new Set(["undefined", "NaN", "Infinity", "__dirname", "__filename"]);
/** Keys through which a value reaches `Function`, a prototype, or its own accessors. */
const REFLECTIVE_KEYS = new Set([
  "constructor", "prototype", "__proto__", "__defineGetter__", "__defineSetter__", "__lookupGetter__", "__lookupSetter__",
]);
/**
 * Keys JavaScript calls by itself: when it turns an object into a primitive or JSON, and `then` when it awaits a value
 * (ESLint awaits the exported config).
 */
const COERCION_KEYS = new Set(["toString", "valueOf", "toJSON", "toLocaleString", "then"]);

/** Throws where `file` runs code, or exports (as `exported`) an object that may hold another config. */
export function refuseCode(file: FlatConfigFile, exported: t.Expression): void {
  const code = new CodeFinder(file).code();
  if (code !== undefined) {
    const line = file.text.slice(0, code.start!).split("\n").length;
    throw new LintConfigFormatError(`it runs code Rerail does not read as data (line ${line}: ${file.source(code)})`);
  }
  // ESLint loads a TypeScript config through Jiti and takes `config.default ?? config`: an exported object that may
  // hold `default` may be read as that instead.
  const top = file.unwrap(exported, 0);
  if (top.type === "ObjectExpression"
    && top.properties.some(property => property.type === "SpreadElement" || keyName(property.key, property.computed) === "default")) {
    throw new LintConfigFormatError("it exports an object that may hold a `default` config, which ESLint's loader can take instead");
  }
}

/** Finds the first part of a file that is code rather than data. */
class CodeFinder {
  constructor(readonly file: FlatConfigFile) {}

  /** The first statement or expression of the file that is code rather than data (see the module comment). */
  code(): t.Node | undefined {
    for (const statement of this.file.program.body) {
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
        const value = this.file.moduleExports(statement.expression);
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
    const declared = statement.declaration;
    if (declaresNoValue(declared)) return undefined;
    return declared.type === "VariableDeclaration" ? this.#declarationCode(declared) : declared;
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
    if (this.file.isGlobal(node.name)) return DATA_GLOBALS.has(node.name) ? undefined : node;
    // A code-running Node module is code wherever it is named: a value read from it can run code when coerced.
    const from = this.file.imported(node.name)?.module;
    if (isCodeModule(from)) return node;
    return given && this.#callMayChange(node.name, from) ? node : undefined;
  }

  /** The file's own binding, or one from a module named by path: that may be the config itself, whose export a call may change. */
  #callMayChange(name: string, from: string | undefined): boolean {
    return this.file.isLocal(name) || isPathModuleName(from);
  }

  #memberCode(node: t.MemberExpression | t.OptionalMemberExpression, given: boolean): t.Node | undefined {
    // `.constructor` of any function is `Function`; a key Rerail cannot read may be that one.
    const key = keyName(node.property, node.computed);
    if (key === undefined || REFLECTIVE_KEYS.has(key)) return node;
    // The environment's variables are strings: reading them reaches nothing else of the process.
    if (key === "env" && this.file.isGlobalProcess(node.object)) return undefined;
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
    const trusted = helper ? "package" : this.file.trustedCall(node);
    if (trusted === undefined) return node;
    if (given && requiresPathModule(node)) return node;
    return this.#calleeCode(node, trusted) ?? this.#argumentsCode(node, given, helper);
  }

  /**
   * The first argument of a call that is code. A config helper changes nothing it is handed, so its arguments are
   * given only where the helper's call itself is; any other call may change any binding of the file handed to it.
   */
  #argumentsCode(node: t.CallExpression | t.OptionalCallExpression | t.NewExpression, given: boolean, helper: boolean): t.Node | undefined {
    return this.#firstNonData(node.arguments, given || !helper);
  }

  /** A call of a config helper (`FlatConfigFile.isConfigHelper`) or of `globalIgnores`: none changes what it is given. */
  #isConfigHelperCall(node: t.CallExpression | t.OptionalCallExpression | t.NewExpression): boolean {
    return (node.type === "CallExpression" && this.file.isConfigHelper(node.callee)) || this.file.isGlobalIgnoresCall(node);
  }

  #calleeCode(node: t.CallExpression | t.OptionalCallExpression | t.NewExpression, trusted: "global" | "package"): t.Node | undefined {
    return trusted === "global" ? undefined : this.#nonData(node.callee, true);
  }
}

const isPathModuleName = (module: string | undefined): boolean => module !== undefined && isPathModule(module);

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
