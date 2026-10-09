// Source facts of an ESLint-based tool (ESLint's own rules, or one plugin's): `upstream/<tool>/source.json`, in the
// format rerail/conformance/languages/pack.ts reads (`upstreamSourceSchema`), which checks 7 to 9 judge a port with.
// Shared by every ESLint pack's recorder (packs/<id>/upstream/**/generate.mjs). Run offline by a maintainer, against
// the pinned upstream checkout and the pinned tools; never in CI.
//
// What upstream says about each rule, at the pinned commit:
// - `implementation` and `tests`: the rule's module (the pack's implementation pattern) and the upstream test files
//   whose RuleTester runs test it.
// - `deprecated`: the rule's `meta.deprecated`, as upstream words it.
// - `enabledBy`: the first approved configuration (conformance.json `tools[].recommended`, in order) that turns the
//   rule on, with the options it sets; null when none does. A configuration's later entries override earlier ones.
// - `names` and `regexes`: the names the rule's own module checks (every word of its strings outside the `meta` block,
//   regex words, object keys), and those of the modules it delegates to: its imports, followed transitively by
//   relative path and into the manifest's helper packages, the ESLint rule it extends, and the helpers of the parser
//   the approved configurations lint with. Names every rule shares are `sharedNames`.
// - `unscored`: the rule's upstream test cases the recorder's fixtures leave out, and why (the recorder says).
// And about the tool: the file `extensions` the approved configurations lint, the language's `keywords` (the pinned
// TypeScript's), its `commentVocabulary` (TypeScript's JSDoc tags and pragmas, ESLint's directive comments and the
// values they take), `testWords`, every word of the code and expected output of every case of every upstream test file
// and of the source files upstream's tests load, and `configurationWords`, the other words of those cases (options,
// settings, globals and expected messages) and of the other files the tests load.
import { execFileSync } from "node:child_process";
import { createHash } from "node:crypto";
import { existsSync, mkdirSync, readdirSync, readFileSync, realpathSync, writeFileSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, extname, join, matchesGlob, relative, resolve, sep } from "node:path";
import { fileURLToPath } from "node:url";

/** The Rerail checkout this recorder runs from. */
const REPOSITORY = dirname(dirname(dirname(fileURLToPath(import.meta.url))));

const order = (left, right) => (left < right ? -1 : left > right ? 1 : 0);
/** A set written in `compareDeterministicStrings` order (UTF-16 code units) without repeats or empty strings. */
const sortedSet = values => [...new Set(values)].filter(value => value !== "").sort(order);

// ---------------------------------------------------------------------------------------------------------------
// Pins: the recorder refuses an upstream that is not the pinned source.

const git = (directory, ...args) => execFileSync("git", ["-C", directory, ...args], { encoding: "utf8" }).trim();
const sha256 = data => createHash("sha256").update(data).digest("hex");

/** Every file under a directory, depth first, in name order. */
function filesUnder(directory) {
  if (!existsSync(directory)) return [];
  const found = [];
  for (const entry of readdirSync(directory, { withFileTypes: true }).sort((a, b) => order(a.name, b.name))) {
    const path = join(directory, entry.name);
    if (entry.isDirectory()) found.push(...filesUnder(path));
    else if (entry.isFile()) found.push(path);
  }
  return found;
}

/** A digest of every file under a directory, by relative path and content. */
function treeDigest(directory) {
  return sha256(filesUnder(directory).map(path => `${relative(directory, path)}\0${sha256(readFileSync(path))}`).join("\n"));
}

/**
 * The version a source file declares for a package: the string an object literal in it maps the package's name to
 * (facebook/react's ReactVersions.js, `'eslint-plugin-react-hooks': '7.1.1'`); undefined when none does.
 */
export function declaredVersion(ts, source, packageName) {
  let found;
  const visit = node => {
    if (found !== undefined) return;
    if (ts.isPropertyAssignment(node) && ts.isObjectLiteralExpression(node.parent) && (ts.isStringLiteral(node.name) || ts.isIdentifier(node.name)) && node.name.text === packageName && ts.isStringLiteral(node.initializer)) {
      found = node.initializer.text;
      return;
    }
    ts.forEachChild(node, visit);
  };
  visit(ts.createSourceFile("versions.js", source, ts.ScriptTarget.ESNext, true, ts.ScriptKind.JS));
  return found;
}

/**
 * Refuses, by throwing, an upstream other than the pinned one:
 * - the checkout is not at `commit`, or its working tree is modified;
 * - the source tree the package is published from (`release.path`) differs between `commit` and the release `tag`;
 * - the installed package, or the version the source at `commit` declares (its package.json, or `release.versionFile`),
 *   is not `version`;
 * - for a package published verbatim (`release.installed`), the installed files differ from `release.path`'s;
 * - for ESLint's own rules, the installed @eslint/js (eslint:recommended) is not `packages/js` at `commit`.
 */
export function assertUpstream({ checkout, commit, tag, version, tools, plugin, eslintPackage = "eslint", release }) {
  const head = git(checkout, "rev-parse", "HEAD");
  if (head !== commit) throw new Error(`${checkout} is at ${head}; pinned ${commit}`);
  if (git(checkout, "status", "--porcelain") !== "") throw new Error(`${checkout} has a modified working tree`);
  const commitTree = git(checkout, "rev-parse", `${commit}:${release.path}`);
  const tagTree = git(checkout, "rev-parse", `refs/tags/${tag}^{commit}:${release.path}`);
  if (commitTree !== tagTree) throw new Error(`${release.path} is tree ${commitTree} at ${commit} and ${tagTree} at ${tag}`);
  const require = createRequire(join(tools, "package.json"));
  const installedPackage = plugin === null ? eslintPackage : plugin.package;
  const installed = JSON.parse(readFileSync(join(tools, "node_modules", installedPackage, "package.json"), "utf8")).version;
  if (installed !== version) throw new Error(`installed ${installedPackage} is ${installed}; pinned ${version}`);
  const packageRoot = dirname(release.path);
  const packageId = plugin === null ? "eslint" : plugin.package;
  const sourceVersion =
    release.versionFile === undefined
      ? JSON.parse(readFileSync(join(checkout, packageRoot, "package.json"), "utf8")).version
      : declaredVersion(require("typescript"), readFileSync(join(checkout, release.versionFile), "utf8"), packageId);
  if (sourceVersion !== version) throw new Error(`the source at ${commit} declares ${packageId} ${sourceVersion}; pinned ${version}`);
  if (release.installed !== undefined && treeDigest(join(tools, "node_modules", release.installed)) !== treeDigest(join(checkout, release.path)))
    throw new Error(`the installed ${release.installed} differs from ${release.path} at ${commit}`);
  if (plugin === null) {
    const js = JSON.parse(readFileSync(join(checkout, "packages", "js", "package.json"), "utf8")).version;
    const installedJs = JSON.parse(readFileSync(join(tools, "node_modules", "@eslint", "js", "package.json"), "utf8")).version;
    if (installedJs !== js || treeDigest(join(tools, "node_modules", "@eslint", "js", "src")) !== treeDigest(join(checkout, "packages", "js", "src")))
      throw new Error(`the installed @eslint/js ${installedJs} differs from packages/js (${js}) at ${commit}`);
  }
}

// ---------------------------------------------------------------------------------------------------------------
// Configurations and rule metadata.

/**
 * The configuration a package's `configs` export holds under a name, read as a user writes it: `flat/recommended` is
 * one key, `flat.recommended` the `recommended` member of `configs.flat` (eslint-plugin-react, eslint-plugin-react-hooks).
 */
export function configNamed(configs, name) {
  return name.split(".").reduce((value, key) => (value !== null && typeof value === "object" ? value[key] : undefined), configs);
}

export function isOff(setting) {
  const severity = Array.isArray(setting) ? setting[0] : setting;
  return severity === "off" || severity === 0;
}

/**
 * The first approved configuration, in order, that turns the rule on, with the options it sets; null when none does.
 * A configuration's later entries override its earlier ones, as ESLint merges a flat config array.
 */
export function firstApproved(approved, ruleId) {
  for (const { name, configs } of approved) {
    const rules = Object.assign({}, ...configs.map(entry => entry.rules ?? {}));
    const setting = rules[ruleId];
    if (setting !== undefined && !isOff(setting)) return { configuration: name, options: Array.isArray(setting) ? setting.slice(1) : [] };
  }
  return null;
}

/** Upstream's own deprecation notice for a rule (`meta.deprecated`, a flag or ESLint's DeprecatedInfo), or null. */
export function deprecationText(meta) {
  const deprecated = meta?.deprecated;
  if (deprecated === undefined || deprecated === false) return null;
  if (deprecated === true) return "deprecated";
  const replaced = (deprecated.replacedBy ?? []).map(entry => entry.rule?.name).filter(name => typeof name === "string");
  const details = [deprecated.deprecatedSince === undefined ? "" : `since ${deprecated.deprecatedSince}`, replaced.length === 0 ? "" : `replaced by ${replaced.join(", ")}`].filter(Boolean);
  return `${(deprecated.message ?? "deprecated").replace(/\.$/u, "")}${details.length === 0 ? "" : ` (${details.join("; ")})`}`;
}

/** The manifest's implementation path of a rule: `{rule}` is its name, `{Rule}` its name in PascalCase (`RulesOfHooks`). */
export function implementationPath(pattern, name) {
  const pascal = name
    .split(/[-_]/u)
    .map(part => part.charAt(0).toUpperCase() + part.slice(1))
    .join("");
  return pattern.replaceAll("{rule}", name).replaceAll("{Rule}", pascal);
}

// ---------------------------------------------------------------------------------------------------------------
// Words, as the TypeScript compiler of the pinned tools reads JavaScript and TypeScript.

const WORD = /[A-Za-z_$][\w$]*/gu;

export function words(text) {
  return text.match(WORD) ?? [];
}

/** The words of a regex's source that name something: escapes, classes and group syntax removed. */
export function regexWords(pattern) {
  const stripped = pattern
    // An escaped backslash before a letter matches a source escape sequence such as `\n`, not a word.
    .replace(/\\\\(?:u\{[0-9A-Fa-f]+\}|u[0-9A-Fa-f]{4}|x[0-9A-Fa-f]{2}|[A-Za-z])/gu, " ")
    .replace(/\[(?:\\.|[^\]\\])*\]/gu, " ")
    .replace(/\(\?(?:P?<[A-Za-z_]\w*>|[:=!]|<[=!]|[a-z]+[:)])/gu, " ")
    .replace(/\\k<[A-Za-z_]\w*>/gu, " ")
    .replace(/\\(?:x\{[0-9A-Fa-f]+\}|x[0-9A-Fa-f]{2}|u\{[0-9A-Fa-f]+\}|u[0-9A-Fa-f]{4}|[pP]\{[^}]*\}|.)/gu, " ");
  // An unescaped `$` is an anchor, so regex words start with a letter or underscore.
  return stripped.match(/[A-Za-z_]\w*/gu) ?? [];
}

/** TypeScript's keywords, read from the pinned compiler: syntax wherever a rule spells them. */
export function keywords(ts) {
  const found = new Set();
  for (let kind = ts.SyntaxKind.FirstKeyword; kind <= ts.SyntaxKind.LastKeyword; kind += 1) {
    const token = ts.tokenToString(kind);
    if (token !== undefined) found.add(token);
  }
  return found;
}

/**
 * The words TypeScript gives meaning to inside comments, read from the pinned compiler: the JSDoc tag names its
 * parser recognizes and its `///` pragmas with their arguments.
 */
export function commentVocabulary(ts, compilerSource) {
  const found = new Set();
  const switches = [...compilerSource.matchAll(/switch \(tagName\.escapedText\) \{/gu)];
  for (const match of switches) {
    const body = compilerSource.slice(match.index, compilerSource.indexOf("default:", match.index));
    for (const tag of body.matchAll(/case "([A-Za-z]+)":/gu)) found.add(tag[1]);
  }
  if (switches.length === 0) throw new Error("cannot find the JSDoc tag names in the pinned TypeScript compiler");
  for (const [pragma, definition] of Object.entries(ts.commentPragmas)) {
    for (const word of words(pragma)) found.add(word);
    for (const argument of definition.args ?? []) for (const word of words(argument.name)) found.add(word);
  }
  return found;
}

/** The string literals of a source that are single words, in source order: `"off"`, `"warn"`, not `"use strict"`. */
function stringWords(ts, node, found = []) {
  if (ts.isStringLiteral(node) && /^[A-Za-z_]\w*$/u.test(node.text)) found.push(node.text);
  ts.forEachChild(node, child => void stringWords(ts, child, found));
  return found;
}

/**
 * The values ESLint's directive comments take, from the pinned eslint's sources: a global's setting in
 * `/* global a:off *\/` (the case labels of source-code.js's normalizeConfigGlobal) and a rule's severity in
 * `/* eslint semi: "warn" *\/` (the strings of shared/severity.js).
 */
export function directiveValuesFrom(ts, sourceCode, severity) {
  const settings = [];
  const visit = node => {
    if (ts.isFunctionDeclaration(node) && node.name?.text === "normalizeConfigGlobal") {
      const cases = child => {
        if (ts.isCaseClause(child) && ts.isStringLiteral(child.expression)) settings.push(child.expression.text);
        ts.forEachChild(child, cases);
      };
      cases(node);
    } else ts.forEachChild(node, visit);
  };
  visit(ts.createSourceFile("source-code.js", sourceCode, ts.ScriptTarget.ESNext, true, ts.ScriptKind.JS));
  const severities = stringWords(ts, ts.createSourceFile("severity.js", severity, ts.ScriptTarget.ESNext, true, ts.ScriptKind.JS));
  if (settings.length === 0 || severities.length === 0) throw new Error("cannot find the global settings (normalizeConfigGlobal) or the severities (shared/severity.js) in the pinned eslint");
  return sortedSet([...settings, ...severities]);
}

function directiveValues(ts, eslintDirectory) {
  const read = path => readFileSync(join(eslintDirectory, path), "utf8");
  return directiveValuesFrom(ts, read("lib/languages/js/source-code/source-code.js"), read("lib/shared/severity.js"));
}

/**
 * The words of TypeScript source as its scanner sees them: identifiers, keywords, and the words inside strings,
 * templates, comments and JSX text. Numbers contribute nothing, so `100n` and `0x1F` yield no words.
 */
export function sourceWords(ts, text, jsx) {
  const scanner = ts.createScanner(ts.ScriptTarget.ESNext, false, jsx ? ts.LanguageVariant.JSX : ts.LanguageVariant.Standard, text);
  const kinds = ts.SyntaxKind;
  const found = [];
  for (let token = scanner.scan(); token !== kinds.EndOfFileToken; token = scanner.scan()) {
    if (token === kinds.NumericLiteral || token === kinds.BigIntLiteral) continue;
    if (token === kinds.Identifier || token === kinds.PrivateIdentifier || (token >= kinds.FirstKeyword && token <= kinds.LastKeyword)) found.push(scanner.getTokenText().replace(/^#/u, ""));
    else found.push(...words(scanner.getTokenText()));
  }
  return found;
}

/** The script kind a path's extension gives a file; anything else reads as TypeScript. */
function scriptKind(ts, path) {
  const extension = extname(path).toLowerCase();
  if (extension === ".tsx") return ts.ScriptKind.TSX;
  if (extension === ".jsx") return ts.ScriptKind.JSX;
  if ([".js", ".mjs", ".cjs"].includes(extension)) return ts.ScriptKind.JS;
  return ts.ScriptKind.TS;
}

/**
 * The documentation in a rule's `meta` block (its docs, option schema, type and the like): every member but
 * `defaultOptions`, whose values are the options the rule runs with when a configuration sets none, and `messages`,
 * the text the rule reports.
 */
const IMPLEMENTED_META = new Set(["defaultOptions", "messages"]);

export function ruleDocumentation(ts) {
  const members = [ts.SyntaxKind.PropertyAssignment, ts.SyntaxKind.ShorthandPropertyAssignment, ts.SyntaxKind.SpreadAssignment, ts.SyntaxKind.MethodDeclaration, ts.SyntaxKind.GetAccessor, ts.SyntaxKind.SetAccessor];
  return node => {
    const object = node.parent;
    const owner = object?.parent;
    return members.includes(node.kind) && !IMPLEMENTED_META.has(node.name?.text ?? "") && object?.kind === ts.SyntaxKind.ObjectLiteralExpression && owner?.kind === ts.SyntaxKind.PropertyAssignment && owner?.name?.text === "meta";
  };
}

/** Every node of each file outside its documentation blocks, depth first. */
function visitSources(ts, files, isDocumentation, visitor) {
  for (const file of files) {
    const sourceFile = ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.ESNext, true, scriptKind(ts, file));
    const visit = node => {
      if (isDocumentation(node)) return;
      visitor(node);
      for (const child of node.getChildren()) visit(child);
    };
    visit(sourceFile);
  }
}

/** The regex literals of sources outside their documentation blocks, as JavaScript regexes (without `g`). */
export function literalRegexes(ts, files, isDocumentation) {
  const found = [];
  visitSources(ts, files, isDocumentation, node => {
    if (node.kind === ts.SyntaxKind.RegularExpressionLiteral && node.text !== undefined) {
      const end = node.text.lastIndexOf("/");
      try {
        const regex = new RegExp(node.text.slice(1, end), node.text.slice(end + 1).replace("g", ""));
        found.push({ source: regex.source, flags: regex.flags });
      } catch {
        // A literal TypeScript accepts but this runtime rejects explains nothing.
      }
    }
  });
  return found;
}

/**
 * Names a source checks: the words of string literals without whitespace (a name, or a token the code matches against,
 * such as `javascript:` or `eslint-disable-line`; prose has spaces), quoted words in other strings, and regex words.
 * With `everyStringWord`, every word of every string counts: for sources whose prose all sits in the documentation
 * block, as an ESLint rule's messages sit in `meta`, so the strings left are data the code builds, such as the
 * `'parameter property'` a message names. With `objectKeys`, the keys of object literals count too: the lookup tables
 * a source checks names against, such as ESLint's `{ Math: { pow: { [CALL]: true } } }` trace map or conf/globals.js's
 * `{ Array: false, ... }`.
 */
export function literalNames(ts, files, isDocumentation, options = {}) {
  const names = new Set();
  const literals = [ts.SyntaxKind.StringLiteral, ts.SyntaxKind.NoSubstitutionTemplateLiteral, ts.SyntaxKind.TemplateHead, ts.SyntaxKind.TemplateMiddle, ts.SyntaxKind.TemplateTail];
  const members = [ts.SyntaxKind.PropertyAssignment, ts.SyntaxKind.ShorthandPropertyAssignment, ts.SyntaxKind.MethodDeclaration, ts.SyntaxKind.GetAccessor, ts.SyntaxKind.SetAccessor];
  visitSources(ts, files, isDocumentation, node => {
    if (literals.includes(node.kind) && node.text !== undefined) {
      if (options.everyStringWord === true || !/\s/u.test(node.text)) for (const word of words(node.text)) names.add(word);
      for (const match of node.text.matchAll(/["'`]([A-Za-z_$][\w$]*)["'`]/gu)) names.add(match[1]);
    }
    // A quoted key is a string literal, read above; a computed key is an expression.
    if (options.objectKeys === true && members.includes(node.kind) && node.parent?.kind === ts.SyntaxKind.ObjectLiteralExpression && node.name?.kind === ts.SyntaxKind.Identifier && node.name?.text !== undefined) names.add(node.name.text);
    if (node.kind === ts.SyntaxKind.RegularExpressionLiteral && node.text !== undefined) {
      for (const word of regexWords(node.text.slice(1, node.text.lastIndexOf("/")))) names.add(word);
    }
  });
  return names;
}

// ---------------------------------------------------------------------------------------------------------------
// The modules a rule delegates to.

/** The npm package a bare module specifier names: `@scope/name` or `name`, without a subpath. */
export function packageName(specifier) {
  const parts = specifier.split("/");
  return parts.slice(0, specifier.startsWith("@") ? 2 : 1).join("/");
}

/** The file a relative module specifier names, as Node and TypeScript resolve it from `directory`. */
export function resolveModule(directory, specifier) {
  const base = resolve(directory, specifier);
  return [base, `${base}.ts`, `${base}.js`, join(base, "index.ts"), join(base, "index.js")].find(candidate => existsSync(candidate) && extname(candidate) !== "");
}

/** The helper package a resolved module belongs to: the innermost `node_modules/<helper>/` on its path. */
export function helperPackage(file, helpers) {
  const path = file.replaceAll("\\", "/");
  let found;
  for (const name of helpers) {
    const at = path.lastIndexOf(`/node_modules/${name}/`);
    if (at !== -1 && (found === undefined || at > found.at)) found = { name, at };
  }
  return found === undefined ? undefined : path.slice(0, found.at + `/node_modules/${found.name}/`.length);
}

/**
 * Follows a rule's imports through upstream's checkout and the tools' installed packages. The checkout has no
 * node_modules of its own: a bare specifier in a checkout file resolves from the tools directory (`toolsRequire`), as
 * it would through a node_modules link to it.
 */
export function moduleGraph(ts, checkout, toolsRequire) {
  const parse = file => ts.createSourceFile(file, readFileSync(file, "utf8"), ts.ScriptTarget.ESNext, true, file.endsWith(".ts") ? ts.ScriptKind.TS : ts.ScriptKind.JS);
  const hasModifier = (node, kind) => ts.canHaveModifiers(node) && (ts.getModifiers(node) ?? []).some(modifier => modifier.kind === kind);
  const hasExport = node => hasModifier(node, ts.SyntaxKind.ExportKeyword);

  /**
   * Where a module specifier leads from the importing file: a relative path as Node and TypeScript resolve it, or a
   * module of a helper package as Node resolves it; undefined for any other package.
   */
  const resolveSpecifier = (file, specifier, helpers) => {
    if (specifier.startsWith(".")) return resolveModule(dirname(file), specifier);
    if (!helpers.has(packageName(specifier))) return undefined;
    try {
      return (file.startsWith(checkout + sep) ? toolsRequire : createRequire(file)).resolve(specifier);
    } catch {
      return undefined;
    }
  };

  /** The module and exported name an import declaration binds to a local name (`import x from "m"` binds m's default). */
  const importOf = (source, local) => {
    for (const statement of source.statements) {
      if (!ts.isImportDeclaration(statement) || !ts.isStringLiteral(statement.moduleSpecifier) || statement.importClause === undefined) continue;
      const clause = statement.importClause;
      if (clause.name?.text === local) return { specifier: statement.moduleSpecifier.text, name: "default" };
      const element = clause.namedBindings !== undefined && ts.isNamedImports(clause.namedBindings) ? clause.namedBindings.elements.find(entry => entry.name.text === local) : undefined;
      if (element !== undefined) return { specifier: statement.moduleSpecifier.text, name: (element.propertyName ?? element.name).text };
    }
    return undefined;
  };

  const bindingNames = name => (ts.isIdentifier(name) ? [name.text] : name.elements.flatMap(element => (ts.isBindingElement(element) ? bindingNames(element.name) : [])));

  /** The specifier of `require("x")`, also wrapped as TypeScript's CommonJS output wraps it (`__importStar(require("x"))`). */
  const requiredSpecifier = node => {
    if (!ts.isCallExpression(node)) return undefined;
    if (ts.isIdentifier(node.expression) && node.expression.text === "require" && node.arguments.length === 1 && ts.isStringLiteral(node.arguments[0])) return node.arguments[0].text;
    if (ts.isIdentifier(node.expression) && /^__import(Star|Default)$/u.test(node.expression.text) && node.arguments.length === 1) return requiredSpecifier(node.arguments[0]);
    return undefined;
  };

  /** `exports.name` or `module.exports.name`: the name a CommonJS module exports by assignment. */
  const exportsMember = node => {
    if (!ts.isPropertyAccessExpression(node)) return undefined;
    const target = node.expression;
    const isExports = (ts.isIdentifier(target) && target.text === "exports") || (ts.isPropertyAccessExpression(target) && ts.isIdentifier(target.expression) && target.expression.text === "module" && target.name.text === "exports");
    return isExports ? node.name.text : undefined;
  };

  /**
   * The file that defines a name a module exports: the module itself when it declares the export, otherwise the module
   * it re-exports the name from, followed through ES re-exports and CommonJS builds (`exports.name =`,
   * `Object.defineProperty(exports, "name", { get })`, `__exportStar(require(...), exports)`, and
   * `exports.name = require(...)`, a namespace the required module defines). A JSON module's default export is its data.
   */
  const definingModule = (file, name, helpers = new Set(), seen = new Set()) => {
    if (seen.has(file)) return undefined;
    seen.add(file);
    if (name === "default" && extname(file) === ".json") return file;
    const source = parse(file);
    const follow = (specifier, exported) => {
      const target = resolveSpecifier(file, specifier, helpers);
      return target === undefined ? undefined : definingModule(target, exported, helpers, seen);
    };
    for (const statement of source.statements) {
      if (name === "default" && ts.isExportAssignment(statement) && !statement.isExportEquals) {
        const imported = ts.isIdentifier(statement.expression) ? importOf(source, statement.expression.text) : undefined;
        return imported === undefined ? file : (follow(imported.specifier, imported.name) ?? file);
      }
      if (name === "default" && hasExport(statement) && hasModifier(statement, ts.SyntaxKind.DefaultKeyword)) return file;
      if (ts.isExportDeclaration(statement) && statement.moduleSpecifier === undefined && statement.exportClause !== undefined && ts.isNamedExports(statement.exportClause) && statement.exportClause.elements.some(element => element.name.text === name)) return file;
      if (hasExport(statement)) {
        if (ts.isVariableStatement(statement) && statement.declarationList.declarations.some(declaration => bindingNames(declaration.name).includes(name))) return file;
        if ((ts.isFunctionDeclaration(statement) || ts.isClassDeclaration(statement) || ts.isEnumDeclaration(statement) || ts.isInterfaceDeclaration(statement) || ts.isTypeAliasDeclaration(statement)) && statement.name?.text === name) return file;
      }
    }
    // CommonJS: `exports.name = value` defines the name here (TypeScript's `exports.a = exports.b = void 0` only declares
    // it); `exports.name = require(...)` exports that module as a namespace.
    let assigned;
    const findAssignment = node => {
      if (assigned !== undefined) return;
      if (ts.isBinaryExpression(node) && node.operatorToken.kind === ts.SyntaxKind.EqualsToken && exportsMember(node.left) === name) {
        let value = node.right;
        while (ts.isBinaryExpression(value) && value.operatorToken.kind === ts.SyntaxKind.EqualsToken && exportsMember(value.left) !== undefined) value = value.right;
        if (!ts.isVoidExpression(value)) {
          const namespace = requiredSpecifier(value);
          assigned = namespace === undefined ? file : (resolveSpecifier(file, namespace, helpers) ?? null);
          return;
        }
      }
      ts.forEachChild(node, findAssignment);
    };
    findAssignment(source);
    if (assigned !== undefined) return assigned ?? undefined;
    // What CommonJS bindings hold a required module: `var util_1 = require("./util")`.
    const required = new Map();
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
        const element = named?.find(entry => entry.name.text === name);
        if (named !== undefined && element === undefined) continue;
        const found = follow(statement.moduleSpecifier.text, element?.propertyName?.text ?? name);
        if (found !== undefined) return found;
      }
      if (!ts.isExpressionStatement(statement) || !ts.isCallExpression(statement.expression)) continue;
      const call = statement.expression;
      const callee = call.expression;
      // __exportStar(require("./x"), exports)
      if (ts.isIdentifier(callee) && callee.text === "__exportStar" && call.arguments.length >= 1) {
        const specifier = requiredSpecifier(call.arguments[0]);
        const found = specifier === undefined ? undefined : follow(specifier, name);
        if (found !== undefined) return found;
      }
      // Object.defineProperty(exports, "name", { get: function () { return x_1.original; } })
      if (
        ts.isPropertyAccessExpression(callee) &&
        callee.getText(source) === "Object.defineProperty" &&
        call.arguments.length === 3 &&
        ts.isIdentifier(call.arguments[0]) &&
        call.arguments[0].text === "exports" &&
        ts.isStringLiteral(call.arguments[1]) &&
        call.arguments[1].text === name &&
        ts.isObjectLiteralExpression(call.arguments[2])
      ) {
        let returned;
        const findReturn = node => {
          if (returned !== undefined) return;
          if (ts.isReturnStatement(node) && node.expression !== undefined && ts.isPropertyAccessExpression(node.expression) && ts.isIdentifier(node.expression.expression)) {
            returned = { binding: node.expression.expression.text, exported: node.expression.name.text };
            return;
          }
          ts.forEachChild(node, findReturn);
        };
        findReturn(call.arguments[2]);
        const specifier = returned === undefined ? undefined : required.get(returned.binding);
        const found = specifier === undefined ? undefined : follow(specifier, returned.exported);
        if (found !== undefined) return found;
      }
    }
    // A CommonJS module's default export is what it assigns to module.exports: the module's own.
    return name === "default" ? file : undefined;
  };

  /** The names read from a binding as `binding.name` (and TypeScript's `(0, binding.name)(...)`), in source order. */
  const namesRead = (source, binding) => {
    const read = new Set();
    const collect = node => {
      if (ts.isPropertyAccessExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === binding) read.add(node.name.text);
      ts.forEachChild(node, collect);
    };
    collect(source);
    return [...read].sort(order);
  };

  /**
   * The modules one file imports: by relative path outside `excluded`, and from helper packages. A relative `require`
   * delegates to the whole module; from a helper package, a `require` delegates to the names destructured from it or
   * read from the binding that holds it.
   */
  const directImports = (file, excluded, helpers) => {
    const source = parse(file);
    const modules = new Set();
    const use = (specifier, names) => {
      const target = resolveSpecifier(file, specifier, helpers);
      if (target === undefined || (excluded !== null && dirname(target) === excluded)) return;
      const defining = names === null ? [] : names.map(name => definingModule(target, name, helpers));
      // A module that defines a name it is asked for, or is asked for nothing by name, is delegated to as a whole.
      if (names === null || defining.includes(target)) modules.add(target);
      for (const defined of defining) if (defined !== undefined) modules.add(defined);
    };
    const requiredNames = call => {
      let node = call;
      while (ts.isCallExpression(node.parent) && requiredSpecifier(node.parent) !== undefined) node = node.parent;
      const parent = node.parent;
      if (ts.isPropertyAccessExpression(parent) && parent.expression === node) return [parent.name.text];
      if (!ts.isVariableDeclaration(parent) || parent.initializer !== node) return null;
      if (ts.isObjectBindingPattern(parent.name)) {
        return parent.name.elements.map(element => (element.propertyName !== undefined && ts.isIdentifier(element.propertyName) ? element.propertyName.text : ts.isIdentifier(element.name) ? element.name.text : "")).filter(Boolean);
      }
      if (!ts.isIdentifier(parent.name)) return null;
      const read = namesRead(source, parent.name.text);
      return read.length === 0 ? null : read;
    };
    const visit = node => {
      if (ts.isImportDeclaration(node) && ts.isStringLiteral(node.moduleSpecifier) && !node.importClause?.isTypeOnly) {
        const bindings = node.importClause?.namedBindings;
        if (bindings !== undefined && ts.isNamedImports(bindings)) {
          use(
            node.moduleSpecifier.text,
            bindings.elements.filter(element => !element.isTypeOnly).map(element => (element.propertyName ?? element.name).text),
          );
        } else if (bindings !== undefined && ts.isNamespaceImport(bindings)) {
          // The names a rule reads from a namespace import are the helpers it calls.
          use(node.moduleSpecifier.text, namesRead(source, bindings.name.text));
        } else if (node.importClause?.name !== undefined) {
          // A default export is the module's own; a helper package's is defined by the module its entry exports it from.
          use(node.moduleSpecifier.text, node.moduleSpecifier.text.startsWith(".") ? null : ["default"]);
        }
      } else if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && node.expression.text === "require" && node.arguments.length === 1 && ts.isStringLiteral(node.arguments[0])) {
        const specifier = node.arguments[0].text;
        use(specifier, specifier.startsWith(".") ? null : requiredNames(node));
      }
      ts.forEachChild(node, visit);
    };
    visit(source);
    return [...modules].sort(order);
  };

  /**
   * The modules an entry module delegates to (a rule's implementation, or a parser an approved configuration sets),
   * followed through what they import in turn: each module imported or required by relative path, other rules aside
   * (modules directly in `excluded`), and from a module that only re-exports, the modules that define the names
   * imported from it. A name imported from a helper package counts through the module that defines it; that module's
   * imports of its own package are its internals and are not followed, and of another helper package only data (a
   * JSON module) counts.
   */
  const importedModules = (implementation, helpers = new Set(), excluded = dirname(implementation)) => {
    const own = new Set();
    const helperModules = new Set();
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
    return { own: [...own].sort(order), helpers: [...helperModules].sort(order) };
  };

  return { definingModule, importedModules };
}

// ---------------------------------------------------------------------------------------------------------------
// Upstream's test files.

/** Every string a value holds, depth first: the values of a message's `data`, whatever their nesting. */
function strings(value) {
  if (typeof value === "string") return [value];
  if (Array.isArray(value)) return value.flatMap(strings);
  if (value !== null && typeof value === "object") return Object.values(value).flatMap(strings);
  return [];
}

/** A path as JSON spells it inside a string. */
const inJson = path => JSON.stringify(path).slice(1, -1);

/**
 * JSON text with every path under one of `roots` (absolute directories) written relative to that root: `<root>/tests`
 * as `tests`, the root itself as `.`. Longer roots go first, so `/private/tmp/c` is not read as `/tmp/c` inside it.
 */
function rootRelative(json, roots) {
  return [...roots].sort((left, right) => right.length - left.length).reduce((text, root) => text.replaceAll(inJson(`${root}${sep}`), "").replaceAll(inJson(root), "."), json);
}

/**
 * Everything a RuleTester case holds for the test words: the code it lints and the code it expects after fixes
 * (`output`, one string or one per fix pass, and each suggestion's output), read as code; and its test configuration
 * (options, settings, the names of the globals it declares) with what it expects reported (each error's message, the
 * values of its `data`, its suggestions' descriptions and data), read as words. Upstream's tests build some options
 * from their own location (`path.resolve(import.meta.dirname, "../../")`); paths under `roots` (upstream's checkout,
 * as given and as its real path) are written checkout-relative, so the words are upstream's on every machine.
 */
export function caseWords(raw, roots = []) {
  const item = typeof raw === "string" ? { code: raw } : (raw ?? {});
  const errors = Array.isArray(item.errors) ? item.errors : [];
  const objects = entries => entries.filter(entry => entry !== null && typeof entry === "object");
  const suggestions = objects(errors).flatMap(error => objects(Array.isArray(error.suggestions) ? error.suggestions : []));
  const outputs = [item.output, ...suggestions.map(suggestion => suggestion.output)].flatMap(output => (Array.isArray(output) ? output : [output]));
  const reported = [...errors, ...suggestions].flatMap(entry => (typeof entry === "string" ? [entry] : entry === null ? [] : [entry.message, entry.desc, ...strings(entry.data)]));
  const filename = typeof item.filename === "string" ? item.filename : "";
  return {
    code: typeof item.code === "string" ? item.code : "",
    outputs: [...new Set(outputs.filter(output => typeof output === "string"))],
    jsx: item.languageOptions?.parserOptions?.ecmaFeatures?.jsx === true || /\.[jt]sx$/u.test(filename),
    configuration: rootRelative(JSON.stringify([item.options ?? null, item.settings ?? null, Object.keys(item.languageOptions?.globals ?? {}), reported.filter(text => typeof text === "string")]), roots),
  };
}

/**
 * The first of `directories` (the recording machine's) that a case's words, as `caseWords` returns them, still name;
 * undefined when none does.
 */
export function localDirectory(words, directories) {
  return directories.find(directory => [words.code, ...words.outputs].some(text => text.includes(directory)) || words.configuration.includes(inJson(directory)));
}

/** The literal directories a glob starts with, so only those are walked. */
function globRoot(glob) {
  const parts = glob.split("/");
  const wild = parts.findIndex(part => /[*?[{]/u.test(part));
  return parts.slice(0, wild === -1 ? parts.length - 1 : wild).join("/");
}

/** The checkout files that match the globs (checkout-relative, posix), in path order. */
export function checkoutFiles(checkout, globs) {
  const matches = path => globs.some(glob => matchesGlob(relative(checkout, path).split(sep).join("/"), glob));
  return sortedSet(globs.flatMap(glob => filesUnder(join(checkout, globRoot(glob))).filter(matches)).map(path => relative(checkout, path).split(sep).join("/")));
}

/** Test fixture files are read as source code when they have one of these extensions, and as plain words otherwise. */
const SOURCE_EXTENSIONS = new Set([".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".cts", ".tsx"]);

// ---------------------------------------------------------------------------------------------------------------
// The facts.

/**
 * How an ESLint rule's sources name what they check: its prose documentation sits in `meta`, so every word of the
 * strings left counts, and so do the keys of its lookup tables.
 */
const NAMES = { everyStringWord: true, objectKeys: true };

/** ESLint's directive comment names, as the pinned eslint's directive pattern accepts them. */
const DIRECTIVES = ["eslint", "eslint-env", "eslint-enable", "eslint-disable", "eslint-disable-line", "eslint-disable-next-line", "exported", "global", "globals"];

/**
 * The source facts of one tool, as `upstream/<tool>/source.json` holds them.
 * - `tools`: the directory holding the pinned tools (node_modules), `checkout` upstream's checkout at `commit`.
 * - `plugin`: null for ESLint's own rules, else `{ package, namespace }`.
 * - `configurations`: conformance.json's `tools[].recommended`, named as `configNamed` reads them from @eslint/js's or
 *   the plugin's `configs`.
 * - `rules`: the upstream rule ids the pack ports, as conformance.json's `upstream.rule` spells them.
 * - `implementation`: the checkout path of a rule's module, `{rule}` (or `{Rule}`) its name without the namespace.
 * - `helpers`: the npm packages whose modules count as upstream's code where upstream's code imports them.
 * - `runs`: every RuleTester run of every upstream test file, `{ testFile, rule, tests: { valid, invalid } }`, with
 *   `testFile` checkout-relative and `rule` the rule the run tests: the rule object the run was given, which is the
 *   tested rule when it is one of the installed tool's rules, or the rule's name (without the namespace) where the
 *   recorder's stubs know it; any other rule (null, or an object that is not the tool's) is no rule of the tool.
 * - `testFixtures`: globs of the checkout files upstream's tests load (read for the test words).
 * - `unscored`: `{ rule, test, case, reason }` for every case of a ported rule's upstream tests that the recorder's
 *   fixtures leave out (`UNSCORED_REASONS` in conformance/languages/pack.ts), `rule` spelled as in `rules`.
 */
export function sourceFacts({ tool, version, repository, commit, tag, tools, checkout, plugin, eslintPackage = "eslint", configurations, rules, implementation, helpers, runs, testFixtures = [], unscored }) {
  const require = createRequire(join(tools, "package.json"));
  const ts = require("typescript");
  const root = realpathSync(checkout);
  const eslintDirectory = dirname(require.resolve(`${eslintPackage}/package.json`));
  const fromEslint = path => require(join(eslintDirectory, path));
  const interop = module => (module && module.__esModule && module.default ? module.default : module);
  const pluginModule = plugin === null ? null : interop(require(plugin.package));
  const upstreamRules = plugin === null ? new Map(require(`${eslintPackage}/use-at-your-own-risk`).builtinRules) : new Map(Object.entries(pluginModule.rules));
  const configSource = plugin === null ? interop(require("@eslint/js")).configs : pluginModule.configs;
  const approved = configurations.map(name => {
    const config = configNamed(configSource, name);
    if (config === undefined) throw new Error(`${plugin === null ? "@eslint/js" : plugin.package} has no configuration named ${name}`);
    return { name, configs: Array.isArray(config) ? config : [config] };
  });
  const bare = name => (plugin !== null && name.startsWith(`${plugin.namespace}/`) ? name.slice(plugin.namespace.length + 1) : name);
  const ruleId = name => (plugin === null ? name : `${plugin.namespace}/${name}`);

  // Every rule reads the scopes the approved configurations' parser builds: the helper modules that parser calls
  // (typescript-eslint's parser calls @typescript-eslint/scope-manager's analyze).
  // The file types they lint are the ones a file matches some configuration's `files` for, after ESLint's defaults.
  const { FlatConfigArray } = fromEslint("lib/config/flat-config-array.js");
  const parsers = new Set();
  const extensions = new Set();
  for (const { configs } of approved) {
    const array = new FlatConfigArray(configs, { basePath: "/corpus" });
    array.normalizeSync();
    for (const extension of [".js", ".mjs", ".cjs", ".jsx", ".ts", ".mts", ".cts", ".tsx"]) {
      const config = array.getConfig(`/corpus/src/example${extension}`);
      if (config !== undefined) extensions.add(extension);
      const parser = config?.languageOptions?.parser;
      if (parser === undefined || parser === null) continue;
      const holds = exports => exports === parser || exports?.default === parser || ["parseForESLint", "parse"].some(key => typeof parser[key] === "function" && exports?.[key] === parser[key]);
      for (const module of Object.values(require.cache)) if (module !== undefined && holds(module.exports)) parsers.add(realpathSync(module.filename));
    }
  }
  const helperSet = new Set(helpers);
  const graph = moduleGraph(ts, root, require);
  const parserHelpers = [...parsers].sort(order).flatMap(parser => graph.importedModules(parser, helperSet, null).helpers);

  const implementationFile = name => {
    const path = join(root, implementationPath(implementation, name));
    return existsSync(path) ? path : undefined;
  };
  const sharedModules = name => {
    const file = implementationFile(name);
    const own = file === undefined ? { own: [], helpers: [] } : graph.importedModules(file, helperSet);
    // A rule that extends an ESLint rule runs that rule's implementation in the installed eslint, and what it imports.
    const base = upstreamRules.get(name)?.meta?.docs?.extendsBaseRule;
    const baseName = base === true ? name : typeof base === "string" ? base : null;
    const baseRule = baseName === null ? undefined : join(eslintDirectory, "lib", "rules", `${baseName}.js`);
    const inherited = baseRule !== undefined && existsSync(baseRule) ? graph.importedModules(baseRule, helperSet) : undefined;
    return sortedSet([...own.own, ...own.helpers, ...(inherited === undefined ? [] : [baseRule, ...inherited.own, ...inherited.helpers]), ...parserHelpers]);
  };

  const isMeta = ruleDocumentation(ts);
  const names = new Map([...upstreamRules].map(([name, rule]) => [rule, name]));
  const testFiles = new Map();
  for (const run of runs) {
    const name = typeof run.rule === "string" ? run.rule : (names.get(run.rule) ?? null);
    if (name === null) continue;
    if (!testFiles.has(name)) testFiles.set(name, new Set());
    testFiles.get(name).add(run.testFile);
  }
  const perRule = new Map();
  for (const id of sortedSet(rules)) {
    const name = bare(id);
    const rule = upstreamRules.get(name);
    const file = implementationFile(name);
    if (rule === undefined && file === undefined) throw new Error(`${id}: neither ${plugin === null ? "eslint" : plugin.package} nor ${implementationPath(implementation, name)} has the rule`);
    const shared = sharedModules(name);
    perRule.set(id, {
      implementation: file === undefined ? [] : [relative(root, file).split(sep).join("/")],
      tests: sortedSet(testFiles.get(name) ?? []),
      deprecated: rule === undefined ? null : deprecationText(rule.meta),
      enabledBy: firstApproved(approved, ruleId(name)),
      implementationNames: file === undefined ? new Set() : literalNames(ts, [file], isMeta, NAMES),
      sharedNames: literalNames(ts, shared, isMeta, NAMES),
      regexes: literalRegexes(ts, [...(file === undefined ? [] : [file]), ...shared], isMeta),
    });
  }
  // The names every ported rule's delegated modules check, and each rule's own remainder.
  const everyRule = [...perRule.values()].map(entry => entry.sharedNames);
  const common = sortedSet([...everyRule[0]].filter(name => everyRule.every(names => names.has(name))));
  const commonSet = new Set(common);

  // The test words: the code and outputs of every case of every upstream test file, and the source files they load;
  // the rest of each case (its configuration and expected messages) and the other files they load are configuration.
  // Paths under the checkout are read checkout-relative; a case that still names the recording machine's checkout,
  // tools or Rerail checkout stops the recorder, as those words differ from machine to machine.
  const roots = [...new Set([root, resolve(checkout)])];
  const local = [...new Set([...roots, resolve(tools), realpathSync(tools), REPOSITORY, realpathSync(REPOSITORY)])];
  const testWords = new Set();
  const configurationWords = new Set();
  for (const run of runs) {
    for (const item of [...(Array.isArray(run.tests?.valid) ? run.tests.valid : []), ...(Array.isArray(run.tests?.invalid) ? run.tests.invalid : [])]) {
      const found = caseWords(item, roots);
      const named = localDirectory(found, local);
      if (named !== undefined) throw new Error(`${run.testFile}: a case names ${named}, a directory of the recording machine, which source.json must not record`);
      for (const code of [found.code, ...found.outputs]) for (const word of sourceWords(ts, code, found.jsx)) testWords.add(word);
      for (const word of words(found.configuration)) configurationWords.add(word);
    }
  }
  for (const file of checkoutFiles(root, testFixtures)) {
    const text = readFileSync(join(root, file), "utf8");
    const extension = extname(file);
    if (SOURCE_EXTENSIONS.has(extension)) for (const word of sourceWords(ts, text, /x$/u.test(extension))) testWords.add(word);
    else for (const word of words(text)) configurationWords.add(word);
  }
  const unscoredOf = new Map(rules.map(id => [id, []]));
  for (const { rule, ...entry } of unscored) {
    if (!unscoredOf.has(rule)) throw new Error(`an unscored test names ${rule}, which the pack does not port`);
    unscoredOf.get(rule).push(entry);
  }

  const { directivesPattern } = fromEslint("lib/shared/directives.js");
  const directives = DIRECTIVES.filter(directive => directivesPattern.test(`${directive} `));
  return {
    tool,
    version,
    repository,
    commit,
    tag,
    configurations,
    extensions: sortedSet(extensions),
    keywords: sortedSet(keywords(ts)),
    // TypeScript's JSDoc tags and pragmas, ESLint's directive comments (`eslint-disable-next-line`, `global`) and the
    // values its directives take (`/* global a:off */`, `/* eslint semi: "warn" */`).
    commentVocabulary: sortedSet([
      ...commentVocabulary(ts, readFileSync(require.resolve("typescript"), "utf8")),
      ...directives.flatMap(directive => directive.split("-")),
      ...directiveValues(ts, eslintDirectory),
    ]),
    testWords: sortedSet(testWords),
    configurationWords: sortedSet([...configurationWords].filter(word => !testWords.has(word))),
    sharedNames: common,
    rules: Object.fromEntries(
      [...perRule].map(([id, entry]) => [
        id,
        {
          implementation: entry.implementation,
          tests: entry.tests,
          deprecated: entry.deprecated,
          enabledBy: entry.enabledBy,
          names: { implementation: sortedSet(entry.implementationNames), shared: sortedSet([...entry.sharedNames].filter(name => !commonSet.has(name))) },
          regexes: [...new Map(entry.regexes.map(regex => [`${regex.flags}/${regex.source}`, regex])).values()],
          unscored: unscoredTests(unscoredOf.get(id)),
        },
      ]),
    ),
  };
}

/** Unscored tests in `compareDeterministicStrings` order of test file, then case; each once. */
export function unscoredTests(entries) {
  const key = entry => `${entry.test}\u0000${entry.case ?? ""}`;
  return [...new Map(entries.map(entry => [key(entry), { test: entry.test, case: entry.case ?? null, reason: entry.reason }])).entries()].sort(([left], [right]) => order(left, right)).map(([, entry]) => entry);
}

/** Writes `source.json` with a two-space indent. */
export function writeSource(path, facts) {
  mkdirSync(dirname(path), { recursive: true });
  writeFileSync(path, `${JSON.stringify(facts, null, 2)}\n`);
}
