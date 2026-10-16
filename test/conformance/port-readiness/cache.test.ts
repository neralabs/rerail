import assert from "node:assert/strict";
import { execFileSync } from "node:child_process";
import { existsSync, mkdtempSync, readFileSync, rmSync, statSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join, posix } from "node:path";
import { build } from "esbuild";
import ts from "typescript";
import { afterAll, describe, test } from "vitest";

import { PACKAGE_ROOT } from "../../../src/engine/index.js";
import { dependencyDigest, GATE_CODE, GATE_PACKAGES, lockedClosure } from "../../../conformance/port-readiness/cache.js";
import { ENGINE_SOURCES } from "../../../conformance/port-readiness/engine.js";

const tracked = (pathspec: readonly string[]): string[] =>
  execFileSync("git", ["ls-files", "-z", "--", ...pathspec], { cwd: PACKAGE_ROOT, encoding: "utf8", maxBuffer: 1 << 28 }).split("\0").filter((path) => path !== "");

/**
 * Every file `entry` reaches through relative imports, by repository path: `x.js` is the `x.ts` beside its importer, as
 * tsx loads it. A relative specifier that resolves to no file fails, so a resolution this misreads cannot quietly
 * shrink the graph the key is checked against.
 */
function importGraph(entry: string): Set<string> {
  const reached = new Set<string>();
  const queue = [entry];
  while (queue.length > 0) {
    const file = queue.pop()!;
    if (reached.has(file)) continue;
    reached.add(file);
    for (const { fileName } of ts.preProcessFile(readFileSync(join(PACKAGE_ROOT, file), "utf8"), true, true).importedFiles) {
      if (!fileName.startsWith("./") && !fileName.startsWith("../")) continue;
      const target = posix.join(posix.dirname(file), fileName.endsWith(".js") ? `${fileName.slice(0, -3)}.ts` : fileName);
      const absolute = join(PACKAGE_ROOT, target);
      assert.ok(existsSync(absolute) && statSync(absolute).isFile(), `${file} imports ${fileName}, which resolves to no file`);
      queue.push(target);
    }
  }
  return reached;
}

describe("the gate code a rule's key covers", () => {
  const graph = importGraph("conformance/port-readiness/run.ts");
  const keyed = new Set(tracked(GATE_CODE));
  const excluded = tracked(["conformance/port-readiness"]).filter((path) => !keyed.has(path));

  test("run.ts's import graph is read whole", () => {
    for (const path of ["conformance/port-readiness/checks/matches-real-code.ts", "conformance/port-readiness/corpus.ts", "conformance/port-readiness/cache.ts"]) {
      assert.ok(graph.has(path), `run.ts's import graph misses ${path}`);
    }
  });

  // An excluded file run.ts reaches could change a verdict while every cached result of it stays standing.
  test("run.ts reaches no file the key leaves out", () => {
    assert.ok(excluded.length > 0, "GATE_CODE excludes nothing");
    for (const path of excluded) assert.ok(!graph.has(path), `${path} is out of the key, but run.ts imports it`);
  });

  // The lists are JSON, which no import reaches: a gate that read one by path would be judging with an unkeyed file.
  test("no gate code run.ts reaches names the not-gated or pending list", () => {
    for (const path of [...graph].filter((path) => path.startsWith("conformance/port-readiness/"))) {
      const lines = readFileSync(join(PACKAGE_ROOT, path), "utf8").split("\n").filter((line) => !line.includes(":(exclude)"));
      for (const name of ["ungated-rules.json", "pending.json"]) assert.ok(!lines.some((line) => line.includes(name)), `${path} names ${name}`);
    }
  });

  test("the run's options and the files run.ts reads by path stay in the key", () => {
    for (const path of ["conformance/port-readiness/cli.ts", "conformance/port-readiness/corpus-tasks.json"]) assert.ok(keyed.has(path), `${path} is out of the key`);
  });

  // A misspelt exclusion excludes nothing, and the file it meant stays in every key without anyone noticing.
  test("every exclusion names a tracked file", () => {
    for (const entry of GATE_CODE.filter((entry) => entry.startsWith(":(exclude)"))) {
      const path = entry.slice(":(exclude)".length);
      assert.deepEqual(tracked([path]), [path], `${entry} names no tracked file`);
    }
  });
});

type Packages = Record<string, Record<string, unknown>>;

const lockOf = (packages: Packages, lockfileVersion = 3) => ({ name: "app", lockfileVersion, requires: true, packages: { "": { name: "app" }, ...packages } });

describe("the locked closure of a package-lock.json", () => {
  // b's c is neither in b's own node_modules nor at the top: npm finds it one level up, in a's.
  test("a dependency is the copy nearest its dependent, as npm finds it", () => {
    const closure = lockedClosure(lockOf({
      "node_modules/a": { version: "1.0.0", integrity: "sha512-a", dependencies: { b: "^2.0.0" } },
      "node_modules/a/node_modules/b": { version: "2.0.0", integrity: "sha512-b2", dependencies: { c: "^2.0.0" } },
      "node_modules/a/node_modules/c": { version: "2.0.0", resolved: "https://registry.npmjs.org/c/-/c-2.0.0.tgz" },
      "node_modules/b": { version: "1.0.0", integrity: "sha512-b1" },
      "node_modules/c": { version: "1.0.0", integrity: "sha512-c1" },
    }), ["a"]);
    assert.deepEqual(closure, new Map([
      ["node_modules/a", "1.0.0\0sha512-a"],
      ["node_modules/a/node_modules/b", "2.0.0\0sha512-b2"],
      ["node_modules/a/node_modules/c", "2.0.0\0https://registry.npmjs.org/c/-/c-2.0.0.tgz"],
    ]));
  });

  test("a scoped package is found nested and hoisted alike", () => {
    const closure = lockedClosure(lockOf({
      "node_modules/@s/a": { version: "1.0.0", integrity: "sha512-a", dependencies: { "@t/b": "^1.0.0" } },
      "node_modules/@s/a/node_modules/@t/b": { version: "1.0.0", integrity: "sha512-b", dependencies: { c: "^1.0.0" } },
      "node_modules/c": { version: "1.0.0", integrity: "sha512-c" },
    }), ["@s/a"]);
    assert.deepEqual([...closure.keys()].sort(), ["node_modules/@s/a", "node_modules/@s/a/node_modules/@t/b", "node_modules/c"]);
  });

  test("an optional dependency or optional peer the lockfile does not install is skipped; an installed one is kept", () => {
    const closure = lockedClosure(lockOf({
      "node_modules/a": {
        version: "1.0.0",
        integrity: "sha512-a",
        optionalDependencies: { fsevents: "^2.0.0", o: "^1.0.0" },
        peerDependencies: { p: "*", q: "*", r: "*" },
        peerDependenciesMeta: { p: { optional: true }, q: { optional: true } },
      },
      "node_modules/o": { version: "1.0.0", integrity: "sha512-o" },
      "node_modules/p": { version: "1.0.0", integrity: "sha512-p" },
      "node_modules/r": { version: "1.0.0", integrity: "sha512-r" },
    }), ["a"]);
    assert.deepEqual([...closure.keys()].sort(), ["node_modules/a", "node_modules/o", "node_modules/p", "node_modules/r"]);
  });

  test("type packages are skipped, installed or not", () => {
    const closure = lockedClosure(lockOf({
      "node_modules/a": { version: "1.0.0", integrity: "sha512-a", dependencies: { "@types/node": "*", "@types/missing": "*" } },
      "node_modules/@types/node": { version: "1.0.0", integrity: "sha512-types" },
    }), ["a"]);
    assert.deepEqual([...closure.keys()], ["node_modules/a"]);
  });

  test("a required dependency or root the lockfile does not install throws", () => {
    assert.throws(() => lockedClosure(lockOf({ "node_modules/a": { version: "1.0.0", dependencies: { b: "^1.0.0" } } }), ["a"]), /node_modules\/a needs b/u);
    assert.throws(() => lockedClosure(lockOf({ "node_modules/a": { version: "1.0.0", peerDependencies: { b: "*" } } }), ["a"]), /node_modules\/a needs b/u);
    assert.throws(() => lockedClosure(lockOf({}), ["a"]), /installs no a/u);
  });

  // A workspace's sources are tracked files the key does not hash, so a closure that reaches one cannot be keyed
  // by version.
  test("a link into the workspace throws", () => {
    const lock = lockOf({
      "node_modules/a": { version: "1.0.0", dependencies: { w: "*" } },
      "node_modules/w": { resolved: "packs/w", link: true },
      "packs/w": { name: "w", version: "1.0.0" },
    });
    assert.throws(() => lockedClosure(lock, ["a"]), /node_modules\/w links to the workspace packs\/w/u);
  });

  test("a lockfile without a packages map throws", () => {
    assert.throws(() => lockedClosure({ lockfileVersion: 1, dependencies: { a: { version: "1.0.0" } } }, ["a"]), /lockfileVersion 2 or later/u);
    assert.throws(() => lockedClosure(lockOf({ "node_modules/a": { version: "1.0.0" } }, 1), ["a"]), /lockfileVersion 2 or later/u);
    assert.throws(() => lockedClosure(null, ["a"]), /lockfileVersion 2 or later/u);
  });
});

describe("the npm side of a rule's key", () => {
  const roots: string[] = [];
  afterAll(() => {
    for (const root of roots) rmSync(root, { recursive: true, force: true });
  });

  type Manifest = { type?: string; scripts: Record<string, string>; [key: string]: unknown };

  /** The key of a checkout whose package.json and lockfile install every gate package, after `edit` changes them. */
  function digest(edit: (manifest: Manifest, packages: Packages) => void = () => {}): string {
    const root = mkdtempSync(join(tmpdir(), "gates-dependencies-"));
    roots.push(root);
    const manifest = {
      name: "app",
      version: "1.0.0",
      type: "module",
      scripts: { "port-readiness": "node --import tsx conformance/port-readiness/cli.ts", test: "vitest run --dir test" },
    };
    const packages: Packages = Object.fromEntries(GATE_PACKAGES.map((name) => [`node_modules/${name}`, { version: "1.0.0", integrity: `sha512-${name}` }]));
    packages["node_modules/apache-arrow"]!.dependencies = { x: "^1.0.0" };
    packages["node_modules/apache-arrow/node_modules/x"] = { version: "1.0.0", integrity: "sha512-x" };
    packages["node_modules/left-pad"] = { version: "1.0.0", integrity: "sha512-left-pad" };
    edit(manifest, packages);
    writeFileSync(join(root, "package.json"), `${JSON.stringify(manifest, null, 2)}\n`);
    writeFileSync(join(root, "package-lock.json"), `${JSON.stringify(lockOf(packages), null, 2)}\n`);
    return dependencyDigest(root);
  }

  const before = digest();

  test("a package the gates do not load, or a package.json edit off the verdict path, keeps the key", () => {
    assert.equal(digest((_, packages) => Object.assign(packages["node_modules/left-pad"]!, { version: "1.0.1", integrity: "sha512-left-pad-2" })), before);
    assert.equal(digest((manifest) => Object.assign(manifest.scripts, { test: "vitest run" })), before);
    assert.equal(digest((manifest) => Object.assign(manifest, { version: "1.0.1", description: "changed" })), before);
  });

  test("the command CI runs, its pre and post scripts, and how tsx loads the gate code move the key", () => {
    assert.notEqual(digest((manifest) => Object.assign(manifest.scripts, { "port-readiness": "node --import tsx conformance/port-readiness/cli.ts --no-cache" })), before);
    assert.notEqual(digest((manifest) => Object.assign(manifest.scripts, { "preport-readiness": "npm run build:native" })), before);
    assert.notEqual(digest((manifest) => Object.assign(manifest.scripts, { "postport-readiness": "rm -rf .cache" })), before);
    assert.notEqual(digest((manifest) => Object.assign(manifest.scripts, { "build:native": "napi build --platform --manifest-path native/Cargo.toml" })), before);
    assert.notEqual(digest((manifest) => Object.assign(manifest, { type: "commonjs" })), before);
  });

  test("a gate package's version or integrity, or a nested dependency's, moves the key", () => {
    assert.notEqual(digest((_, packages) => Object.assign(packages["node_modules/zod"]!, { version: "1.0.1" })), before);
    assert.notEqual(digest((_, packages) => Object.assign(packages["node_modules/zod"]!, { integrity: "sha512-zod-2" })), before);
    assert.notEqual(digest((_, packages) => Object.assign(packages["node_modules/apache-arrow/node_modules/x"]!, { version: "1.0.1" })), before);
  });
});

/**
 * Files run.ts loads that no rule's key covers: only checks/passes-conformance.ts reaches them, and run.ts judges
 * passes-conformance afresh on every run and never caches it.
 */
const uncachedOnly = (path: string): boolean => path === "conformance/engine.ts" || path.startsWith("conformance/languages/");

// GATE_PACKAGES is a ledger whose gap keeps a stale verdict, so it is checked against what run.ts actually loads.
test("GATE_PACKAGES covers every npm package a bundle of run.ts loads, and names none it does not", async () => {
  const closure = lockedClosure(JSON.parse(readFileSync(join(PACKAGE_ROOT, "package-lock.json"), "utf8")), GATE_PACKAGES);
  const { metafile, warnings } = await build({
    entryPoints: [join(PACKAGE_ROOT, "conformance/port-readiness/run.ts")],
    absWorkingDir: PACKAGE_ROOT,
    bundle: true,
    write: false,
    metafile: true,
    platform: "node",
    format: "esm",
    external: ["*.node"],
    logLevel: "silent",
  });
  // A require or import the bundle leaves out is a warning, and a module loaded that way would escape this check.
  assert.deepEqual(warnings.map((warning) => warning.text), []);
  const slashed = (path: string) => path.replaceAll("\\", "/");
  const inputs = new Map(Object.entries(metafile.inputs).map(([path, input]) => [slashed(path), input.imports.map((imported) => slashed(imported.path))]));

  const reached = new Set<string>();
  for (const path of [...inputs.keys()].filter((path) => path.startsWith("node_modules/"))) {
    const location = /^(.*node_modules\/(?:@[^/]+\/)?[^/]+)\//u.exec(path)?.[1];
    assert.ok(location !== undefined && closure.has(location), `run.ts loads ${path}, outside the locked closure of GATE_PACKAGES`);
    reached.add(location);
  }
  // tsx compiles the gate code rather than being loaded by it.
  for (const name of GATE_PACKAGES.filter((name) => name !== "tsx")) assert.ok(reached.has(`node_modules/${name}`), `run.ts loads no ${name}`);

  const keyed = new Set([...tracked(GATE_CODE), ...tracked(ENGINE_SOURCES)]);
  for (const [path, imports] of [...inputs].filter(([path]) => !path.startsWith("node_modules/"))) {
    assert.ok(keyed.has(path) || uncachedOnly(path), `run.ts loads ${path}, which is in no rule's key`);
    // A module loaded through createRequire is one the bundle cannot see; the native binding it loads in native.ts
    // is out of the key by design.
    if (path !== "src/engine/native/native.ts") {
      assert.ok(!readFileSync(join(PACKAGE_ROOT, path), "utf8").includes("createRequire("), `${path} loads modules through createRequire`);
    }
    for (const imported of imports.filter(uncachedOnly)) {
      assert.ok(uncachedOnly(path) || path === "conformance/port-readiness/checks/passes-conformance.ts", `${path} reaches ${imported}, which only passes-conformance may`);
    }
  }
});
