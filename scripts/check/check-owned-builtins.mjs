#!/usr/bin/env node
// The Node built-ins only one module in src/ may use, each owned by the module that makes it safe on Windows.
//
// Rerail fixes several classes of Windows defect by sending every use of a built-in through one module. Each row of
// RULES names that module (the owner), the built-in modules, the names in them only the owner may reach (or none, when
// no use is safe) or, for a row with `options`, the options only the owner may call them with, the files the rule
// covers, and the reason:
//
//   child-process   node:child_process, anywhere in src/. On Windows Node starts a child with a visible window unless
//                   told otherwise, and a console program (git, node, npm, PowerShell) started by a process with no
//                   console, such as the daemon, opens a console window that Windows Terminal brings to the front. One
//                   missed option opened a window every few seconds while the daemon ran Git.
//   program-path    With child-process: a call to one of its starters that names the program by a literal that is not
//                   an absolute path. Windows looks for such a program in the current folder first, and Rerail runs
//                   inside repositories it does not trust.
//   file-replace    rename and renameSync of node:fs, anywhere in src/. On Windows renaming onto or from a file another
//                   process holds open fails with EPERM, EACCES or EBUSY for a moment; one unretried rename failed the
//                   daemon's start and another left an agent's edit in place after a rewind.
//   checkout-paths  realpath and realpathSync of node:fs, in the modules that key records by a checkout path or look
//                   a caller's folder up in its checkout (KEYED). On Windows the JS realpath keeps the caller's
//                   spelling (`c:\work\app`, VS Code's, or 8.3 short names, `C:\Users\RUNNER~1`) while Git prints
//                   `C:/work/app` with long names, so keys resolved both ways never met and no rule ran in the session.
//   no-follow       constants.O_NOFOLLOW of node:fs, anywhere in src/. Windows has no O_NOFOLLOW, and `flags |
//                   undefined` adds nothing, so an open meant to refuse a symbolic link followed a planted one there.
//   folder-removal  rm of node:fs or node:fs/promises called with maxRetries, or with options the check cannot read
//                   (a spread, a variable), anywhere in src/; any other rm is free. That rm repeats the retries at every
//                   level of the folder, each try at a folder removing its contents with a full set again: a file
//                   Windows held one folder down kept a removal trying for minutes rather than 5.5 s.
//   git-starts      Rerail's own Git starter (runGit) and the path Git is started from (gitExecutable, findGit), and
//                   a starter given a literal path to a git program, anywhere in src/ but git-access/git-process.ts.
//                   Every Git process Rerail starts is one question that file names and counts, so a start written
//                   anywhere else is one nobody counts. src/engine/git.ts may only hand the names on (`export ...
//                   from`), for the two files of src/setup/ that still import them from there.
//
// A rule's modules are built-in module names or, for Rerail's own modules, repository paths: a relative specifier is
// read as the file it names, `./git.js` in src/engine/ as src/engine/git.ts.
//
// One walk of each file finds every way to reach a name: an `import` (named, default or namespace), `import x =
// require(...)`, the module's name as an argument of any call (`require`, `import()`, a `createRequire` result,
// `module.require`, `process.getBuiltinModule`), then member access, destructuring and aliases of those
// (`fs.promises.rename`, `const { renameSync } = fs`, `(await import("node:fs/promises")).rename`). Handing a name on
// is a finding too: `export ... from` the module (named, `*` or `* as`), or a local `export` of a binding that holds
// it. Type-only imports and `typeof import(...)` load nothing.
//
//   node scripts/check/check-owned-builtins.mjs   (npm run check:owned-builtins; part of npm run check)
import { dirname, posix, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";
import { CODE, codeFiles, eachNode, FS_MODULES as FS, lineOf, originalName, parseSource } from "./source-files.mjs";

/** An absolute POSIX path, a drive-letter path or a UNC path. */
const ABSOLUTE = /^(?:\/|[A-Za-z]:[\\/]|\\\\)/u;
const STARTERS = new Set(["spawn", "spawnSync", "execFile", "execFileSync", "spawnShown"]);

/**
 * The modules that key records by a checkout path, or look a caller's folder up in its checkout: whole folders end in
 * `/`. The API is listed whole because each of its modules takes paths from callers.
 */
export const KEYED = [
  "src/engine/catalog/",
  "src/engine/audit/",
  "src/runtime/runtime.ts",
  "src/runtime/presentation-preference-store.ts",
  "src/api/",
];

const inSrc = path => path.startsWith("src/");

/** Where Rerail's Git starter and Git's path live, and the file that hands them on. */
const GIT_MODULES = ["src/engine/git-access/git-process.ts", "src/engine/git-access/find-git.ts", "src/engine/git.ts"];
/**
 * The files of src/ that still reach the Git starter or Git's path through src/engine/git.ts: src/setup/ is a path
 * REVIEW.md asks a person to review, and moves to the questions of git-access/ in a change of its own.
 */
const GIT_STARTS_ELSEWHERE = ["src/setup/harness-registration.ts", "src/setup/preflight.ts"];
const keyed = path => KEYED.some(entry => (entry.endsWith("/") ? path.startsWith(entry) : path === entry));

/**
 * One owned built-in: `names` are dotted paths inside each of `modules` (`constants.O_NOFOLLOW`); without them every
 * value use of the module is the owner's. With `options`, reaching the names is free and only a call of one given any of
 * those options is the owner's. `covers` picks the files the rule reads, the owner aside.
 */
export const RULES = [
  {
    id: "child-process",
    owner: "src/engine/child-process.ts",
    modules: ["node:child_process", "child_process"],
    covers: inSrc,
    message: "loads node:child_process; import spawn, spawnSync, execFile or execFileSync from src/engine/child-process.ts instead, which hides the child's window on Windows",
    programPath: true,
  },
  {
    id: "file-replace",
    owner: "src/engine/file-replace.ts",
    modules: FS,
    names: ["rename", "renameSync"],
    covers: inSrc,
    message: "renames a file directly; use replaceFile, replaceFileSync, renameWithRetry or renameWithRetrySync from src/engine/file-replace.ts, which retry a file Windows holds for a moment",
  },
  {
    id: "checkout-paths",
    owner: "src/engine/catalog/checkout-control.ts",
    modules: FS,
    names: ["realpath", "realpathSync"],
    covers: keyed,
    message: "resolves a path with node:fs's realpath; use checkoutRealpath or physicalPath from src/engine/catalog/checkout-control.ts, which spells a Windows path as Git does",
  },
  {
    id: "no-follow",
    owner: "src/engine/no-follow.ts",
    modules: FS,
    names: ["constants.O_NOFOLLOW"],
    covers: inSrc,
    message: "reads O_NOFOLLOW, which Windows lacks, so the open follows a symbolic link there; open with openNoFollow or openNoFollowSync from src/engine/no-follow.ts, which refuse one on every platform",
  },
  {
    id: "folder-removal",
    owner: "src/engine/file-replace.ts",
    modules: FS,
    names: ["rm"],
    options: ["maxRetries"],
    covers: inSrc,
    message: "gives rm maxRetries, which rm repeats at every level of the folder, so a file Windows holds a folder down stalls the removal for minutes; use removeWithRetry from src/engine/file-replace.ts, which counts its own tries",
  },
  {
    id: "git-starts",
    owner: "src/engine/git-access/git-process.ts",
    modules: GIT_MODULES,
    names: ["runGit", "gitExecutable", "findGit"],
    covers: path => inSrc(path) && !GIT_STARTS_ELSEWHERE.includes(path),
    relays: ["src/engine/git.ts"],
    gitProgram: true,
    message: "starts Git outside src/engine/git-access/git-process.ts, where each Git process is a question it names and counts; ask through checkoutGit or GitReads (src/engine/git-access/checkout-git.ts), or call requireGit to check that Git is installed",
  },
].map(rule => ({ ...rule, names: rule.names?.map(name => name.split(".")) }));

const PROGRAM_PATH_MESSAGE = "names a program without its folder, which Windows looks for in the current folder first; pass an absolute path (windowsSystemProgram, programOnPath)";
const RELAY_MESSAGE = "holds more than `export ... from`: this file only hands git-access/'s names on";

/** `path` inside `module` as a rule names it: a built-in's `default` is the module itself, and fs's `promises` holds the same names. */
function canonical(module, path) {
  const rest = path[0] === "default" ? path.slice(1) : path;
  return (module === "node:fs" || module === "fs") && rest[0] === "promises" ? rest.slice(1) : rest;
}

const startsWith = (path, prefix) => prefix.length <= path.length && prefix.every((part, index) => path[index] === part);

/** Whether `reach` (`{ module, path }`) is exactly a name `rule` owns, or for a rule with no names, the module at all. */
function reaches(rule, reach) {
  if (!rule.modules.includes(reach.module)) return false;
  if (rule.names === undefined) return true;
  const path = canonical(reach.module, reach.path);
  return rule.names.some(name => name.length === path.length && startsWith(name, path));
}

/** Whether handing `reach` on hands on a name `rule` owns: the name itself or anything that holds it. */
function exposes(rule, reach) {
  if (!rule.modules.includes(reach.module)) return false;
  if (rule.names === undefined) return true;
  const path = canonical(reach.module, reach.path);
  return rule.names.some(name => startsWith(name, path));
}

/** The member `key` of what `reach` is. */
const within = (reach, key) => ({ module: reach.module, path: [...reach.path, key] });

const keyText = (name) => (ts.isIdentifier(name) || ts.isStringLiteralLike(name) ? name.text : undefined);

/**
 * The text of a property's name, a computed one's too when it is a string: `a` in `a`, `"a"` and `["a"]`, and none in
 * `[a]`, whose key is the variable's value.
 */
function nameText(name) {
  if (!ts.isComputedPropertyName(name)) return keyText(name);
  return ts.isStringLiteralLike(name.expression) ? name.expression.text : undefined;
}

/** The names in the `{ ... }` of an import or export that are not marked `type`. */
const valueElements = elements => elements.filter(element => !element.isTypeOnly);

/** Whether the `{ ... }` of an import or export brings in values: `{}` loads the module, and so does a value name. */
const namesLoadValues = elements => elements.length === 0 || valueElements(elements).length > 0;

/** Whether an import clause brings in values: a bare import (no clause), a default or namespace import, or `{ ... }`. */
function importLoadsValues(clause) {
  if (clause?.isTypeOnly === true) return false;
  const bindings = clause?.namedBindings;
  return bindings === undefined || ts.isNamespaceImport(bindings) || clause.name !== undefined
    || namesLoadValues(bindings.elements);
}

/** Whether an export declaration hands on values: `*`, `* as m` or `{ ... }`, unless it is `export type`. */
function exportLoadsValues(node) {
  const clause = node.exportClause;
  return !node.isTypeOnly && (clause === undefined || ts.isNamespaceExport(clause) || namesLoadValues(clause.elements));
}

/** Whether an import, export or `import = require` declaration brings in values, not only types. */
function loadsValues(node) {
  if (ts.isImportEqualsDeclaration(node)) return !node.isTypeOnly;
  return ts.isExportDeclaration(node) ? exportLoadsValues(node) : importLoadsValues(node.importClause);
}

/**
 * The module `specifier` names from the file at `path`: a built-in's name as written, and a relative specifier as the
 * repository path of the TypeScript file it resolves to.
 */
function moduleOf(path, specifier) {
  if (!specifier.startsWith(".")) return specifier;
  return posix.normalize(posix.join(posix.dirname(path), specifier)).replace(/\.js$/u, ".ts");
}

/** The module an import, export or `import = require` declaration in the file at `path` names, when it is one of `modules`. */
function declaredModule(node, modules, path) {
  const specifier = ts.isImportEqualsDeclaration(node)
    ? (ts.isExternalModuleReference(node.moduleReference) ? node.moduleReference.expression : undefined)
    : node.moduleSpecifier;
  if (specifier === undefined || !ts.isStringLiteralLike(specifier)) return undefined;
  const module = moduleOf(path, specifier.text);
  return modules.has(module) ? module : undefined;
}

const isImport = node => ts.isImportDeclaration(node) || ts.isImportEqualsDeclaration(node);

/** The names an import clause binds, each with its path in the module: `default`, the module itself or an export's name. */
function clauseBindings(clause) {
  const bindings = clause.namedBindings;
  const named = bindings === undefined ? []
    : ts.isNamespaceImport(bindings) ? [[bindings.name, []]]
    : valueElements(bindings.elements).map(element => [element.name, [originalName(element)]]);
  return clause.name === undefined ? named : [[clause.name, ["default"]], ...named];
}

/** The names an import or `import = require` binds, each with its path in the module; none for `import type`. */
function importBindings(node) {
  if (ts.isImportEqualsDeclaration(node)) return node.isTypeOnly ? [] : [[node.name, []]];
  return node.importClause?.isTypeOnly === false ? clauseBindings(node.importClause) : [];
}

/** The property an object pattern's element reads: `a` in `{ a }`, `{ a: b }`, `{ "a": b }` and `{ ["a"]: b }`. */
const elementKey = element => nameText(element.propertyName ?? element.name);

/** Whether an object literal's property may set one of `names`: a spread, a key the check cannot read, or one of them. */
function propertySets(property, names) {
  if (ts.isSpreadAssignment(property)) return true;
  const key = nameText(property.name);
  return key === undefined || names.includes(key);
}

/** Whether a call's options argument may set one of `names`: options that are no object literal may hold any. */
function optionsSet(options, names) {
  if (options === undefined) return false;
  return !ts.isObjectLiteralExpression(options) || options.properties.some(property => propertySets(property, names));
}

/** The paths an `export ... from` clause hands on: the module itself for `*` and `* as m`, else each value name. */
function reexportedPaths(clause) {
  if (clause === undefined || ts.isNamespaceExport(clause)) return [[]];
  return valueElements(clause.elements).map(element => [originalName(element)]);
}

/** The program a call of a starter names by a literal (`spawn("git", ...)`); undefined for any other call. */
function literalProgram(call) {
  const [command] = call.arguments;
  return ts.isIdentifier(call.expression) && STARTERS.has(call.expression.text)
    && command !== undefined && ts.isStringLiteralLike(command) ? command.text : undefined;
}

/** A program path whose last part is `git` or `git.exe`, in any letter case. */
const GIT_PROGRAM = /(?:^|[\\/])git(?:\.exe)?$/iu;

const isArgument = node => ts.isCallExpression(node.parent) && node.parent.arguments.includes(node);

const exported = node => node.modifiers?.some(modifier => modifier.kind === ts.SyntaxKind.ExportKeyword) === true;

/** The expressions that evaluate to their operand: `(m)`, `await m`, `m!`, `m as T`, `m satisfies T` and `<T>m`. */
const WRAPPERS = new Set([
  ts.SyntaxKind.ParenthesizedExpression,
  ts.SyntaxKind.AwaitExpression,
  ts.SyntaxKind.NonNullExpression,
  ts.SyntaxKind.AsExpression,
  ts.SyntaxKind.SatisfiesExpression,
  ts.SyntaxKind.TypeAssertionExpression,
]);

/** The checks {@link Walk#check} runs at a node of each kind, by method name, in this order. */
const CHECKS = new Map([
  [ts.SyntaxKind.ImportDeclaration, ["checkBindings", "checkLoad"]],
  [ts.SyntaxKind.ImportEqualsDeclaration, ["checkBindings", "checkLoad"]],
  [ts.SyntaxKind.VariableDeclaration, ["checkBindings"]],
  [ts.SyntaxKind.ExportDeclaration, ["checkLoad"]],
  [ts.SyntaxKind.StringLiteral, ["checkLoadingArgument"]],
  [ts.SyntaxKind.NoSubstitutionTemplateLiteral, ["checkLoadingArgument"]],
  [ts.SyntaxKind.PropertyAccessExpression, ["checkMember"]],
  [ts.SyntaxKind.ElementAccessExpression, ["checkMember"]],
  [ts.SyntaxKind.ExportSpecifier, ["checkLocalExport"]],
  [ts.SyntaxKind.ExportAssignment, ["checkExportAssignment"]],
  [ts.SyntaxKind.VariableStatement, ["checkExportedVariables"]],
  [ts.SyntaxKind.CallExpression, ["checkProgram", "checkOptions"]],
]);

/** A file's walk: what each local name reaches inside a watched module, and every finding. */
class Walk {
  constructor(source, rules) {
    this.source = source;
    /** The rules on reaching a module or a name; {@link optionRules} are on the options a call of a name passes. */
    this.rules = rules.filter(rule => rule.options === undefined);
    this.optionRules = rules.filter(rule => rule.options !== undefined);
    this.modules = new Set(rules.flatMap(rule => rule.modules));
    /** @type {Map<string, { module: string, path: string[] }>} */
    this.bindings = new Map();
    /** @type {Map<string, { line: number, rule: string, message: string }>} */
    this.found = new Map();
  }

  report(at, rule, message = rule.message, id = rule.id) {
    const line = lineOf(this.source, at);
    this.found.set(`${line}:${id}`, { line, rule: id, message });
  }

  /** Reports at `at` each rule with names that `test` (`reaches` or `exposes`) finds `reach` breaks, if there is one. */
  reportNamed(test, reach, at) {
    if (reach === undefined) return;
    for (const rule of this.rules) if (rule.names !== undefined && test(rule, reach)) this.report(at, rule);
  }

  /** A value load of `module` breaks every rule that owns the whole module. */
  reportLoad(node, module) {
    for (const rule of this.rules) if (rule.names === undefined && reaches(rule, { module, path: [] })) this.report(node, rule);
  }

  /** Whether `node` is a string naming a watched module. */
  namesModule(node) {
    return ts.isStringLiteralLike(node) && this.modules.has(moduleOf(this.source.fileName, node.text));
  }

  /** What `node` evaluates to inside a watched module, if the walk can tell. */
  resolve(node) {
    switch (node.kind) {
      case ts.SyntaxKind.Identifier: return this.bindings.get(node.text);
      case ts.SyntaxKind.PropertyAccessExpression: return this.member(node.expression, node.name.text);
      case ts.SyntaxKind.ElementAccessExpression: return this.member(node.expression, keyText(node.argumentExpression));
      case ts.SyntaxKind.CallExpression: return this.loaded(node);
      default: return WRAPPERS.has(node.kind) ? this.resolve(node.expression) : undefined;
    }
  }

  /** The member `key` of what `base` evaluates to, when that is inside a watched module and the key is known. */
  member(base, key) {
    const reach = key === undefined ? undefined : this.resolve(base);
    return reach === undefined ? undefined : within(reach, key);
  }

  /** The watched module a call loads by naming it as an argument (`require`, `import()`, `process.getBuiltinModule`). */
  loaded(call) {
    const named = call.arguments.find(argument => this.namesModule(argument));
    return named === undefined ? undefined : { module: moduleOf(this.source.fileName, named.text), path: [] };
  }

  /** What a variable's value reaches inside a watched module, if it has one the walk can tell. */
  valueOf(declaration) {
    return declaration.initializer === undefined ? undefined : this.resolve(declaration.initializer);
  }

  /**
   * Records every binding to a watched module before {@link check} reads a use, so a use above its binding resolves.
   * The walk repeats until it adds none: an alias written above the binding it copies (`const n = m` in a function
   * above `const m = require(...)`) is bound only on the second pass, and a use written above the alias needs it.
   */
  bind() {
    const ignore = () => {};
    for (let size = -1; size !== this.bindings.size;) {
      size = this.bindings.size;
      eachNode(this.source, node => this.bindDeclaration(node, ignore));
    }
  }

  /** Binds what an import, `import = require` or variable declaration names, calling `seen` with each binding. */
  bindDeclaration(node, seen) {
    for (const [name, reach] of this.declared(node)) this.bindPattern(name, reach, seen);
  }

  /** Each name an import, `import = require` or variable binds to a watched module, with what it reaches there. */
  declared(node) {
    if (ts.isVariableDeclaration(node)) return this.initialized(node);
    const module = isImport(node) ? declaredModule(node, this.modules, this.source.fileName) : undefined;
    return module === undefined ? [] : importBindings(node).map(([name, path]) => [name, { module, path }]);
  }

  /** A variable whose value reaches into a watched module (`const m = require("node:fs")`, `const { a } = m`). */
  initialized(declaration) {
    const reach = this.valueOf(declaration);
    return reach === undefined ? [] : [[declaration.name, reach]];
  }

  /** Each name a binding pattern binds, with what it reaches; `visit` also sees every property a pattern reads on the way. */
  bindPattern(name, reach, visit) {
    visit(name, reach);
    if (ts.isIdentifier(name)) this.bindName(name.text, reach);
    else if (ts.isObjectBindingPattern(name)) {
      for (const element of name.elements) this.bindElement(element, reach, visit);
    }
  }

  /** Binds `name` to `reach`, unless an earlier binding holds the name. */
  bindName(name, reach) {
    if (!this.bindings.has(name)) this.bindings.set(name, reach);
  }

  /** One element of an object pattern: a rest element (`...n`) holds the whole object, any other the member it reads. */
  bindElement(element, reach, visit) {
    if (element.dotDotDotToken !== undefined) {
      this.bindPattern(element.name, reach, visit);
      return;
    }
    const key = elementKey(element);
    if (key !== undefined) this.bindPattern(element.name, within(reach, key), visit);
  }

  /** Reports every finding in the file, after {@link bind}: each node gets the checks {@link CHECKS} lists for its kind. */
  check() {
    eachNode(this.source, (node) => {
      for (const name of CHECKS.get(node.kind) ?? []) this[name](node);
    });
  }

  /** An import or variable that binds an owned name: `import { rename } from "node:fs"`, `const { rename } = fs`. */
  checkBindings(node) {
    this.bindDeclaration(node, (_, reach) => this.reportNamed(reaches, reach, node));
  }

  /** An import, export or `import = require` that loads a watched module as a value, and what an export hands on. */
  checkLoad(node) {
    const module = declaredModule(node, this.modules, this.source.fileName);
    if (module === undefined || !loadsValues(node)) return;
    this.reportLoad(node, module);
    if (ts.isExportDeclaration(node)) this.checkReexport(node, module);
  }

  /** `export ... from` a watched module hands on each name it re-exports, and `*` hands on all of them. */
  checkReexport(node, module) {
    for (const path of reexportedPaths(node.exportClause)) this.reportNamed(exposes, { module, path }, node);
  }

  /** A watched module named as an argument of any call loads it, reported at the statement the call is in. */
  checkLoadingArgument(node) {
    if (!this.namesModule(node) || !isArgument(node)) return;
    this.reportLoad(ts.findAncestor(node, ts.isStatement) ?? node, moduleOf(this.source.fileName, node.text));
  }

  /** A member read of an owned name: `fs.rename`, `fs["rename"]`, `fs.promises.rename`. */
  checkMember(node) {
    this.reportNamed(reaches, this.resolve(node), node);
  }

  /** A name handed on from this module: `export { fs }`. */
  checkLocalExport(node) {
    if (node.isTypeOnly || node.parent.parent.moduleSpecifier !== undefined) return;
    this.reportNamed(exposes, this.bindings.get(originalName(node)), node);
  }

  /** A value handed on as this module's default: `export default fs`. */
  checkExportAssignment(node) {
    this.reportNamed(exposes, this.resolve(node.expression), node);
  }

  /** An exported variable that holds a name: `export const p = fs.promises`. */
  checkExportedVariables(node) {
    if (!exported(node)) return;
    for (const declaration of node.declarationList.declarations) {
      this.reportNamed(exposes, this.valueOf(declaration), declaration);
    }
  }

  /** A call of a name a rule with `options` owns, passing one of them: `rm(path, { maxRetries: 10 })`. */
  checkOptions(call) {
    const reach = this.resolve(call.expression);
    if (reach === undefined) return;
    for (const rule of this.optionRules) {
      if (reaches(rule, reach) && optionsSet(call.arguments[1], rule.options)) this.report(call, rule);
    }
  }

  /**
   * A call that names its program by a literal: one without a folder is reported as program-path under each rule with
   * `programPath`, and a git program under each rule with `gitProgram`.
   */
  checkProgram(call) {
    const program = literalProgram(call);
    if (program === undefined) return;
    for (const rule of this.rules) {
      if (rule.programPath === true && !ABSOLUTE.test(program)) this.report(call, rule, PROGRAM_PATH_MESSAGE, "program-path");
      if (rule.gitProgram === true && GIT_PROGRAM.test(program)) this.report(call, rule);
    }
  }
}

/** Each statement of `source` that is not `export ... from`, in a file `rule` lets hand its names on. */
function relayProblems(source, rule) {
  return source.statements
    .filter(statement => !ts.isExportDeclaration(statement) || statement.moduleSpecifier === undefined)
    .map(statement => ({ line: lineOf(source, statement), rule: rule.id, message: RELAY_MESSAGE }));
}

/** The problems of one parsed file: each rule that covers it walks it, and each rule it relays for reads its statements. */
function fileProblems(source, rules) {
  const path = source.fileName;
  const relayed = rules.filter(rule => rule.relays?.includes(path));
  const walk = new Walk(source, rules.filter(rule => rule.owner !== path && !relayed.includes(rule) && rule.covers(path)));
  walk.bind();
  walk.check();
  return [...walk.found.values(), ...relayed.flatMap(rule => relayProblems(source, rule))].sort((a, b) => a.line - b.line);
}

/**
 * Every problem in `files` (`{ path, text }`, repository-relative `/`-separated paths) under `rules`, as `{ path, line,
 * rule, message }`, in file order and then by line. Each file is parsed once, for all the rules that cover it.
 */
export function ownedBuiltinProblems(files, rules = RULES) {
  const problems = [];
  for (const { path, text } of files) {
    if (!CODE.test(path) || !rules.some(rule => rule.owner !== path && rule.covers(path))) continue;
    for (const { line, rule, message } of fileProblems(parseSource(path, text), rules)) problems.push({ path, line, rule, message });
  }
  return problems;
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const files = codeFiles(resolve(dirname(fileURLToPath(import.meta.url)), "..", ".."), "src");
  const problems = ownedBuiltinProblems(files);
  for (const { path, line, rule, message } of problems) console.error(`${path}:${line}: [${rule}] ${message}`);
  console.log(problems.length === 0
    ? `check-owned-builtins: ${files.length} files reach ${RULES.map(rule => rule.id).join(", ")} only through their owners`
    : `check-owned-builtins: ${problems.length} problem(s)`);
  process.exit(problems.length === 0 ? 0 : 1);
}
