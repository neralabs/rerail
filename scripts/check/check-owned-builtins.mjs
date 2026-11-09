#!/usr/bin/env node
// The Node built-ins only one module in src/ may use, each owned by the module that makes it safe on Windows.
//
// Rerail fixes several classes of Windows defect by sending every use of a built-in through one module. Each row of
// RULES names that module (the owner), the built-in modules, the names in them only the owner may reach (or none, when
// no use is safe), the files the rule covers, and the reason:
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
//   checkout-paths  realpath and realpathSync of node:fs, in the modules that key records by a checkout path (KEYED). On
//                   Windows the JS realpath keeps the caller's spelling (`c:\work\app`, VS Code's) while Git prints
//                   `C:/work/app`, so keys resolved both ways never met and no rule ran in the session.
//   no-follow       constants.O_NOFOLLOW of node:fs, anywhere in src/. Windows has no O_NOFOLLOW, and `flags |
//                   undefined` adds nothing, so an open meant to refuse a symbolic link followed a planted one there.
//
// One walk of each file finds every way to reach a name: an `import` (named, default or namespace), `import x =
// require(...)`, the module's name as an argument of any call (`require`, `import()`, a `createRequire` result,
// `module.require`, `process.getBuiltinModule`), then member access, destructuring and aliases of those
// (`fs.promises.rename`, `const { renameSync } = fs`, `(await import("node:fs/promises")).rename`). Handing a name on
// is a finding too: `export ... from` the module (named, `*` or `* as`), or a local `export` of a binding that holds
// it. Type-only imports and `typeof import(...)` load nothing.
//
//   node scripts/check/check-owned-builtins.mjs   (npm run check:owned-builtins; part of npm run check)
import { spawnSync } from "node:child_process";
import { readFileSync } from "node:fs";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

const FS = ["node:fs", "node:fs/promises", "fs", "fs/promises"];
const CODE = /\.(?:[cm]?[jt]s)$/u;
/** An absolute POSIX path, a drive-letter path or a UNC path. */
const ABSOLUTE = /^(?:\/|[A-Za-z]:[\\/]|\\\\)/u;
const STARTERS = new Set(["spawn", "spawnSync", "execFile", "execFileSync", "spawnShown"]);

/** The modules that key records by a checkout path: whole folders end in `/`. */
export const KEYED = [
  "src/engine/catalog/",
  "src/engine/audit/",
  "src/runtime/runtime.ts",
  "src/runtime/presentation-preference-store.ts",
  "src/api/operations.ts",
  "src/api/repository-operations.ts",
];

const inSrc = path => path.startsWith("src/");
const keyed = path => KEYED.some(entry => (entry.endsWith("/") ? path.startsWith(entry) : path === entry));

/**
 * One owned built-in: `names` are dotted paths inside each of `modules` (`constants.O_NOFOLLOW`); without them every
 * value use of the module is the owner's. `covers` picks the files the rule reads, the owner aside.
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
].map(rule => ({ ...rule, names: rule.names?.map(name => name.split(".")) }));

const PROGRAM_PATH_MESSAGE = "names a program without its folder, which Windows looks for in the current folder first; pass an absolute path (windowsSystemProgram, resolveExecutable with binaryOnly)";

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

const keyText = (name) => (ts.isIdentifier(name) || ts.isStringLiteralLike(name) ? name.text : undefined);

/** Whether an import or export declaration brings in values, not only types. */
function loadsValues(node) {
  if (ts.isExportDeclaration(node)) {
    if (node.isTypeOnly) return false;
    const clause = node.exportClause;
    return clause === undefined || ts.isNamespaceExport(clause) || clause.elements.length === 0 || clause.elements.some(element => !element.isTypeOnly);
  }
  const clause = node.importClause;
  if (clause === undefined) return true;
  if (clause.isTypeOnly) return false;
  if (clause.name !== undefined) return true;
  const bindings = clause.namedBindings;
  if (bindings === undefined || ts.isNamespaceImport(bindings)) return true;
  return bindings.elements.length === 0 || bindings.elements.some(element => !element.isTypeOnly);
}

/** The module an import, export or `import = require` declaration names, when it is one of `modules`. */
function declaredModule(node, modules) {
  const specifier = ts.isImportEqualsDeclaration(node)
    ? (ts.isExternalModuleReference(node.moduleReference) ? node.moduleReference.expression : undefined)
    : node.moduleSpecifier;
  return specifier !== undefined && ts.isStringLiteralLike(specifier) && modules.has(specifier.text) ? specifier.text : undefined;
}

/** A file's walk: what each local name reaches inside a watched module, and every finding. */
class Walk {
  constructor(source, rules) {
    this.source = source;
    this.rules = rules;
    this.modules = new Set(rules.flatMap(rule => rule.modules));
    /** @type {Map<string, { module: string, path: string[] }>} */
    this.bindings = new Map();
    /** @type {Map<string, { line: number, rule: string, message: string }>} */
    this.found = new Map();
  }

  line(node) {
    return this.source.getLineAndCharacterOfPosition(node.getStart(this.source)).line + 1;
  }

  report(at, rule, message = rule.message, id = rule.id) {
    const line = this.line(at);
    this.found.set(`${line}:${id}`, { line, rule: id, message });
  }

  /** What `node` evaluates to inside a watched module, if the walk can tell. */
  resolve(node) {
    if (ts.isIdentifier(node)) return this.bindings.get(node.text);
    if (ts.isParenthesizedExpression(node) || ts.isAwaitExpression(node) || ts.isNonNullExpression(node)
      || ts.isAsExpression(node) || ts.isSatisfiesExpression(node) || ts.isTypeAssertionExpression(node)) return this.resolve(node.expression);
    if (ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node)) {
      const key = ts.isPropertyAccessExpression(node) ? node.name.text : keyText(node.argumentExpression);
      const base = key === undefined ? undefined : this.resolve(node.expression);
      return base === undefined ? undefined : { module: base.module, path: [...base.path, key] };
    }
    if (ts.isCallExpression(node)) {
      const loaded = node.arguments.find(argument => ts.isStringLiteralLike(argument) && this.modules.has(argument.text));
      return loaded === undefined ? undefined : { module: loaded.text, path: [] };
    }
    return undefined;
  }

  /** Each name a binding pattern binds, with what it reaches; `visit` also sees every property a pattern reads on the way. */
  bindPattern(name, reach, visit) {
    visit(name, reach);
    if (ts.isIdentifier(name)) {
      if (!this.bindings.has(name.text)) this.bindings.set(name.text, reach);
      return;
    }
    if (!ts.isObjectBindingPattern(name)) return;
    for (const element of name.elements) {
      if (element.dotDotDotToken !== undefined) {
        this.bindPattern(element.name, reach, visit);
        continue;
      }
      const key = element.propertyName === undefined ? keyText(element.name)
        : ts.isComputedPropertyName(element.propertyName) ? keyText(element.propertyName.expression) : keyText(element.propertyName);
      if (key !== undefined) this.bindPattern(element.name, { module: reach.module, path: [...reach.path, key] }, visit);
    }
  }

  /** Records every binding to a watched module, repeated until none is added, so a use above its binding resolves too. */
  bind() {
    const ignore = () => {};
    for (let size = -1; size !== this.bindings.size;) {
      size = this.bindings.size;
      const visit = (node) => {
        this.bindDeclaration(node, ignore);
        ts.forEachChild(node, visit);
      };
      visit(this.source);
    }
  }

  /** Binds what an import, `import = require` or variable declaration names, calling `seen` with each binding. */
  bindDeclaration(node, seen) {
    if (ts.isImportDeclaration(node) && node.importClause !== undefined && !node.importClause.isTypeOnly) {
      const module = declaredModule(node, this.modules);
      if (module === undefined) return;
      const { name, namedBindings } = node.importClause;
      if (name !== undefined) this.bindPattern(name, { module, path: ["default"] }, seen);
      if (namedBindings !== undefined && ts.isNamespaceImport(namedBindings)) this.bindPattern(namedBindings.name, { module, path: [] }, seen);
      if (namedBindings !== undefined && ts.isNamedImports(namedBindings)) {
        for (const element of namedBindings.elements) {
          const key = keyText(element.propertyName ?? element.name);
          if (!element.isTypeOnly && key !== undefined) this.bindPattern(element.name, { module, path: [key] }, seen);
        }
      }
    } else if (ts.isImportEqualsDeclaration(node) && !node.isTypeOnly) {
      const module = declaredModule(node, this.modules);
      if (module !== undefined) this.bindPattern(node.name, { module, path: [] }, seen);
    } else if (ts.isVariableDeclaration(node) && node.initializer !== undefined) {
      const reach = this.resolve(node.initializer);
      if (reach !== undefined) this.bindPattern(node.name, reach, seen);
    }
  }

  /** Reports every finding in the file, after {@link bind}. */
  check() {
    const visit = (node) => {
      this.checkNode(node);
      ts.forEachChild(node, visit);
    };
    visit(this.source);
  }

  checkNode(node) {
    const each = (test, reach, at) => {
      for (const rule of this.rules) if (rule.names !== undefined && test(rule, reach)) this.report(at, rule);
    };
    this.bindDeclaration(node, (_, reach) => each(reaches, reach, node));
    if (ts.isImportDeclaration(node) || ts.isExportDeclaration(node) || ts.isImportEqualsDeclaration(node)) {
      const module = declaredModule(node, this.modules);
      const loads = module !== undefined && (ts.isImportEqualsDeclaration(node) ? !node.isTypeOnly : loadsValues(node));
      if (loads) this.reportLoad(node, module);
      if (loads && ts.isExportDeclaration(node)) this.checkReexport(node, module);
    }
    if (ts.isStringLiteralLike(node) && this.modules.has(node.text) && ts.isCallExpression(node.parent) && node.parent.arguments.includes(node)) {
      this.reportLoad(ts.findAncestor(node, ancestor => ts.isStatement(ancestor)) ?? node, node.text);
    }
    if (ts.isPropertyAccessExpression(node) || ts.isElementAccessExpression(node)) {
      const reach = this.resolve(node);
      if (reach !== undefined) each(reaches, reach, node);
    }
    // A name handed on from this module: `export { fs }`, `export default fs`, `export const p = fs.promises`.
    if (ts.isExportSpecifier(node) && !node.isTypeOnly && node.parent.parent.moduleSpecifier === undefined) {
      const reach = this.bindings.get((node.propertyName ?? node.name).text);
      if (reach !== undefined) each(exposes, reach, node);
    }
    if (ts.isExportAssignment(node)) {
      const reach = this.resolve(node.expression);
      if (reach !== undefined) each(exposes, reach, node);
    }
    if (ts.isVariableStatement(node) && node.modifiers?.some(modifier => modifier.kind === ts.SyntaxKind.ExportKeyword)) {
      for (const declaration of node.declarationList.declarations) {
        const reach = declaration.initializer === undefined ? undefined : this.resolve(declaration.initializer);
        if (reach !== undefined) each(exposes, reach, declaration);
      }
    }
    if (ts.isCallExpression(node) && ts.isIdentifier(node.expression) && STARTERS.has(node.expression.text)) this.checkProgram(node);
  }

  /** A value load of `module` breaks every rule that owns the whole module. */
  reportLoad(node, module) {
    for (const rule of this.rules) if (rule.names === undefined && reaches(rule, { module, path: [] })) this.report(node, rule);
  }

  /** `export ... from` a watched module hands on each name it re-exports, and `*` hands on all of them. */
  checkReexport(node, module) {
    const clause = node.exportClause;
    const paths = clause === undefined || ts.isNamespaceExport(clause) ? [[]]
      : clause.elements.filter(element => !element.isTypeOnly).map(element => [keyText(element.propertyName ?? element.name)]);
    for (const path of paths) {
      for (const rule of this.rules) if (rule.names !== undefined && exposes(rule, { module, path })) this.report(node, rule);
    }
  }

  checkProgram(call) {
    const [command] = call.arguments;
    if (command === undefined || !ts.isStringLiteralLike(command) || ABSOLUTE.test(command.text)) return;
    for (const rule of this.rules) if (rule.programPath === true) this.report(call, rule, PROGRAM_PATH_MESSAGE, "program-path");
  }
}

function parse(path, text) {
  return ts.createSourceFile(path, text, ts.ScriptTarget.Latest, true, /\.[cm]?js$/u.test(path) ? ts.ScriptKind.JS : ts.ScriptKind.TS);
}

/**
 * Every problem in `files` (`{ path, text }`, repository-relative `/`-separated paths) under `rules`, as `{ path, line,
 * rule, message }`, in file order and then by line. Each file is parsed once, for all the rules that cover it.
 */
export function ownedBuiltinProblems(files, rules = RULES) {
  const problems = [];
  for (const { path, text } of files) {
    if (!CODE.test(path)) continue;
    const applying = rules.filter(rule => rule.owner !== path && rule.covers(path));
    if (applying.length === 0) continue;
    const walk = new Walk(parse(path, text), applying);
    walk.bind();
    walk.check();
    const found = [...walk.found.values()].sort((a, b) => a.line - b.line);
    for (const { line, rule, message } of found) problems.push({ path, line, rule, message });
  }
  return problems;
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  // Untracked files too, so a new module is checked before its first commit.
  const listed = spawnSync("git", ["ls-files", "-z", "--cached", "--others", "--exclude-standard", "src"], { cwd: root, encoding: "utf8", maxBuffer: 1 << 28 });
  if (listed.status !== 0) throw new Error(`git ls-files failed: ${listed.stderr}`);
  const files = listed.stdout.split("\0").filter(path => CODE.test(path)).map(path => ({ path, text: readFileSync(resolve(root, path), "utf8") }));
  const problems = ownedBuiltinProblems(files);
  for (const { path, line, rule, message } of problems) console.error(`${path}:${line}: [${rule}] ${message}`);
  console.log(problems.length === 0
    ? `check-owned-builtins: ${files.length} files reach ${RULES.map(rule => rule.id).join(", ")} only through their owners`
    : `check-owned-builtins: ${problems.length} problem(s)`);
  process.exit(problems.length === 0 ? 0 : 1);
}
