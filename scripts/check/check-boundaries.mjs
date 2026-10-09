#!/usr/bin/env node
// The layer boundaries of `src/` (see src/README.md), checked on every relative import with the TypeScript parser.
//
// Rules:
//   engine-isolation      src/engine/ imports no other layer: no host and no service layer.
//   host-isolation        src/hosts/<harness>/ imports no other src/hosts/<harness>/.
//   layer-order           every other layer imports only the layers listed for it in LAYERS (a DAG).
//   src-self-contained    src/ imports nothing outside src/.
//   entry-points          code outside src/ (test/, integration/, conformance/, scripts/, benchmark/) imports src/
//                         only through a layer's index.ts or src/index.ts. Exceptions: unit tests under
//                         test/<layer>/ import their own layer's internals (test/engine/** → src/engine/**,
//                         test/hosts/<harness>/** → src/hosts/<harness>/**), and a vitest mock seam
//                         (vi.mock, vi.doMock, vi.importActual, vi.importMock, with `typeof import()` in its type
//                         arguments) names the module it replaces by path.
//   playground-public-api playground/ imports the package only through its package.json `exports`, i.e. the built
//                         dist/ entry points, never src/ or another dist/ module; its tsconfig `paths` likewise.
import { spawnSync } from "node:child_process";
import { existsSync, readFileSync } from "node:fs";
import { dirname, join, normalize, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import ts from "typescript";

/** Each layer and the layers it may import; `*` is any. A layer always imports its own modules. */
const LAYERS = {
  engine: [],
  runtime: ["engine"],
  api: ["engine", "runtime"],
  presentation: ["engine", "runtime", "api"],
  daemon: ["engine", "runtime", "api"],
  activation: ["engine"],
  harness: ["engine", "runtime", "presentation"],
  setup: ["engine", "runtime", "harness", "activation"],
  host: ["engine", "runtime", "presentation", "harness", "setup"],
  cli: ["*"],
  package: ["*"],
};
const HOSTS = ["omp", "pi", "claude", "codex", "cursor"];
const CODE = /\.(?:[cm]?[jt]s|tsx)$/u;
const OUTSIDE_SRC = ["test/", "integration/", "conformance/", "scripts/", "benchmark/"];
const SKIPPED = ["fixtures/", "test/fixtures/", "packs/"];
const MOCK_SEAMS = new Set(["mock", "doMock", "unmock", "importActual", "importMock"]);

/** `src/<layer>/…` → its layer: `engine`, `hosts/<harness>`, …, or `package` for `src/index.ts`. */
function layerOf(path) {
  const parts = path.split("/");
  if (parts[0] !== "src") return undefined;
  if (parts.length === 2) return "package";
  return parts[1] === "hosts" ? `hosts/${parts[2]}` : parts[1];
}

/** The rule row a layer is checked against: every host shares the `host` row. */
const ruleRow = (layer) => (layer.startsWith("hosts/") ? "host" : layer);

/** The layer whose internals `file` may import as its own unit tests, if any. */
function ownLayerOfTest(file) {
  const parts = file.split("/");
  if (parts[0] !== "test") return undefined;
  if (parts[1] === "hosts") return HOSTS.includes(parts[2]) && parts.length > 3 ? `hosts/${parts[2]}` : undefined;
  return Object.hasOwn(LAYERS, parts[1]) && !["host", "package"].includes(parts[1]) ? parts[1] : undefined;
}

function isEntryPoint(target) {
  return target === "src/index.ts" || /^src\/(?:hosts\/[^/]+|[^/]+)\/index\.ts$/u.test(target);
}

/** Every module specifier in `text` with its line and whether it is a vitest mock seam. */
function specifiers(file, text) {
  const kind = /\.[cm]?js$/u.test(file) ? ts.ScriptKind.JS : file.endsWith(".tsx") ? ts.ScriptKind.TSX : ts.ScriptKind.TS;
  const source = ts.createSourceFile(file, text, ts.ScriptTarget.Latest, false, kind);
  const found = [];
  const add = (node, seam) => {
    if (!ts.isStringLiteralLike(node)) return;
    found.push({ specifier: node.text, line: source.getLineAndCharacterOfPosition(node.getStart(source)).line + 1, seam });
  };
  const visit = (node, seam) => {
    if ((ts.isImportDeclaration(node) || ts.isExportDeclaration(node)) && node.moduleSpecifier !== undefined) add(node.moduleSpecifier, seam);
    else if (ts.isImportEqualsDeclaration(node) && ts.isExternalModuleReference(node.moduleReference)) add(node.moduleReference.expression, seam);
    else if (ts.isImportTypeNode(node) && ts.isLiteralTypeNode(node.argument)) add(node.argument.literal, seam);
    else if (ts.isCallExpression(node)) {
      const callee = node.expression;
      if (callee.kind === ts.SyntaxKind.ImportKeyword || (ts.isIdentifier(callee) && callee.text === "require")) {
        if (node.arguments[0] !== undefined) add(node.arguments[0], seam);
      } else if (ts.isPropertyAccessExpression(callee) && ts.isIdentifier(callee.expression) && callee.expression.text === "vi"
        && MOCK_SEAMS.has(callee.name.text)) {
        if (node.arguments[0] !== undefined) add(node.arguments[0], true);
        for (const argument of node.typeArguments ?? []) visit(argument, true);
        for (const argument of node.arguments.slice(1)) visit(argument, seam);
        return;
      }
    }
    ts.forEachChild(node, child => visit(child, seam));
  };
  visit(source, false);
  return found;
}

/** The repository path a relative specifier names: `.js` resolves to the `.ts` source beside it. */
function targetOf(root, file, specifier) {
  const path = normalize(join(dirname(file), specifier)).replaceAll("\\", "/");
  for (const [from, to] of [[".js", ".ts"], [".mjs", ".mts"], [".js", ".tsx"]]) {
    if (path.endsWith(from) && !existsSync(join(root, path)) && existsSync(join(root, path.slice(0, -from.length) + to))) {
      return path.slice(0, -from.length) + to;
    }
  }
  return path;
}

/** The dist/ files package.json `exports` publishes, as repository paths. */
function publicTargets(root) {
  const manifest = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
  const targets = new Set();
  const collect = (value) => {
    if (typeof value === "string") targets.add(normalize(value).replaceAll("\\", "/"));
    else if (value !== null && typeof value === "object") Object.values(value).forEach(collect);
  };
  collect(manifest.exports);
  return targets;
}

function checkSrc(file, target, line) {
  const from = layerOf(file);
  if (!target.startsWith("src/")) {
    return { file, line, rule: "src-self-contained", message: `src/ imports ${target}, outside src/` };
  }
  const to = layerOf(target);
  if (to === from || to === undefined) return undefined;
  if (from === "engine") return { file, line, rule: "engine-isolation", message: `the engine imports ${to} (${target})` };
  if (from.startsWith("hosts/") && to.startsWith("hosts/")) {
    return { file, line, rule: "host-isolation", message: `${from} imports another host, ${to} (${target})` };
  }
  const allowed = LAYERS[ruleRow(from)];
  if (allowed === undefined) return { file, line, rule: "layer-order", message: `${from} is not a declared layer` };
  if (allowed.includes("*") || allowed.includes(ruleRow(to))) return undefined;
  return {
    file, line, rule: "layer-order",
    message: `${from} may import ${allowed.length === 0 ? "no other layer" : allowed.join(", ")}; it imports ${to} (${target})`,
  };
}

function checkOutside(file, target, line, seam) {
  if (!target.startsWith("src/") || isEntryPoint(target) || seam) return undefined;
  const own = ownLayerOfTest(file);
  if (own !== undefined && layerOf(target) === own) return undefined;
  return {
    file, line, rule: "entry-points",
    message: `imports ${target}; outside src/, import src/${layerOf(target) === "package" ? "" : `${layerOf(target)}/`}index.ts`,
  };
}

function checkPlayground(file, target, line, published) {
  if (target.startsWith("playground/")) return undefined;
  if (target.startsWith("dist/") && published.has(target)) return undefined;
  if (!target.startsWith("src/") && !target.startsWith("dist/")) return undefined;
  return {
    file, line, rule: "playground-public-api",
    message: `imports ${target}; the playground uses only the package's exports (${[...published].filter(path => path.endsWith(".js")).join(", ")})`,
  };
}

/** tsconfig `paths` under playground/ must point at published entry points too. */
function checkPlaygroundPaths(root, file, published) {
  const violations = [];
  const text = readFileSync(join(root, file), "utf8");
  const paths = ts.parseConfigFileTextToJson(file, text).config?.compilerOptions?.paths ?? {};
  for (const [alias, targets] of Object.entries(paths)) {
    for (const value of targets) {
      const target = normalize(join(dirname(file), value)).replaceAll("\\", "/").replace(/\.d\.ts$/u, ".js");
      const violation = checkPlayground(file, target, 1, published);
      const line = text.slice(0, text.indexOf(`"${alias}"`)).split("\n").length;
      if (violation !== undefined) violations.push({ ...violation, line, message: `paths ${alias} → ${value}; ${violation.message.split("; ")[1]}` });
    }
  }
  return violations;
}

function trackedFiles(root) {
  const result = spawnSync("git", ["ls-files", "-z"], { cwd: root, encoding: "utf8" });
  if (result.status !== 0) throw new Error(`git ls-files failed: ${result.stderr}`);
  return result.stdout.split("\0").filter(Boolean).sort();
}

/** Every boundary violation in the checkout at `root`, sorted by file and line. `files`: its tracked files. */
export function checkBoundaries(root, { files = trackedFiles(root) } = {}) {
  const published = publicTargets(root);
  const violations = [];
  for (const file of files) {
    if (SKIPPED.some(prefix => file.startsWith(prefix)) || file.includes("/node_modules/") || !existsSync(join(root, file))) continue;
    const inSrc = file.startsWith("src/");
    const inPlayground = file.startsWith("playground/");
    if (inPlayground && /(?:^|\/)tsconfig[^/]*\.json$/u.test(file)) {
      violations.push(...checkPlaygroundPaths(root, file, published));
      continue;
    }
    if (!CODE.test(file) || (!inSrc && !inPlayground && !OUTSIDE_SRC.some(prefix => file.startsWith(prefix)))) continue;
    for (const { specifier, line, seam } of specifiers(file, readFileSync(join(root, file), "utf8"))) {
      if (!specifier.startsWith(".")) continue;
      const target = targetOf(root, file, specifier);
      const violation = inSrc ? checkSrc(file, target, line)
        : inPlayground ? checkPlayground(file, target, line, published)
        : checkOutside(file, target, line, seam);
      if (violation !== undefined) violations.push(violation);
    }
  }
  violations.sort((a, b) => a.file.localeCompare(b.file) || a.line - b.line || a.rule.localeCompare(b.rule));
  return { violations };
}

if (process.argv[1] !== undefined && resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");
  const { violations } = checkBoundaries(root);
  for (const { file, line, rule, message } of violations) process.stderr.write(`${file}:${line}: [${rule}] ${message}\n`);
  if (violations.length > 0) {
    process.stderr.write(`check-boundaries: ${violations.length} violation(s); the layers are in src/README.md\n`);
    process.exitCode = 1;
  } else {
    process.stdout.write(`check-boundaries: no violations (${relative(process.cwd(), root) || "."})\n`);
  }
}
