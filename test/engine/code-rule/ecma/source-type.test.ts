import { execFileSync } from "node:child_process";
import { mkdir, mkdtemp, readFile, realpath, rm, unlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { AuditInput, AuditRecorder, AuditResult, AuditScope } from "../../../../src/engine/audit/audit.js";
import type { CodeRuleEdit } from "../../../../src/engine/code-rule/code-rule.js";
import { bundledCodeRuleDslDirectory } from "../../../../src/engine/code-rule/code-rule-compile.js";
import { languageFor, NativeCodeRuleEnforcer, sourceFileFacts } from "../../../../src/engine/code-rule/code-rule-enforcer.js";
import { codeRuleSchema } from "../../../../src/engine/code-rule/code-rule-schema.js";
import type { CodeRule } from "../../../../src/engine/code-rule/code-rule-schema.js";
import { loadNativeBinding } from "../../../../src/engine/native/native.js";
import { testCodeRuleCatalog } from "../../../support/repository-rules.js";

// What an ECMA file's nearest package.json gives it: the module system a query that leaves `languageOptions.sourceType`
// unset reads (its `type`), and what `file.package` and `file.jsxRuntime` read. Repository audits (native index and host
// walk), edits and stateless snapshots must all send the same facts, a snapshot holding the package.json files must
// derive them alike, and a changed manifest must change the next result.

const roots: string[] = [];
afterEach(async () => { await Promise.all(roots.splice(0).map(root => rm(root, { recursive: true, force: true }))); });

async function directory(prefix: string): Promise<string> {
  const root = await realpath(await mkdtemp(join(tmpdir(), prefix)));
  roots.push(root);
  return root;
}

type Sources = Record<string, string>;

async function repository(git: boolean, sources: Sources): Promise<string> {
  const root = await directory("rerail-package-facts-");
  for (const [path, source] of Object.entries(sources)) {
    await mkdir(dirname(join(root, path)), { recursive: true });
    await writeFile(join(root, path), source);
  }
  if (git) {
    execFileSync("git", ["init", "--quiet"], { cwd: root });
    execFileSync("git", ["add", "--force", "."], { cwd: root });
    execFileSync("git", ["-c", "user.name=Rerail", "-c", "user.email=rerail@local", "commit", "--quiet", "-m", "seed"], { cwd: root });
  }
  return root;
}

/** Violations per `path code`. */
function counted(violations: ReadonlyArray<{ path?: string | undefined; code: string }>): Record<string, number> {
  const counts: Record<string, number> = {};
  for (const violation of violations) {
    const key = `${violation.path ?? ""} ${violation.code}`;
    counts[key] = (counts[key] ?? 0) + 1;
  }
  return counts;
}

/**
 * An enforcer over `workspaceRoot` with `codeRule` (a function lets it change between edits; snapshots read the first),
 * edits audited in `scope` (touched-files by default), and the three ways a file reaches the engine. `sources` holds
 * what edits and snapshots send for each path; `ecmaPaths` are the files they send.
 */
async function enforcerFor(workspaceRoot: string, codeRule: CodeRule | (() => CodeRule), sources: Sources, scope?: AuditScope) {
  const current = typeof codeRule === "function" ? codeRule : () => codeRule;
  const home = await directory("rerail-package-facts-catalog-");
  const capabilitiesPath = join(home, "capabilities.json");
  await writeFile(capabilitiesPath, await readFile(join(bundledCodeRuleDslDirectory(), "capabilities.json")));
  const audits = new Map<string, AuditInput & { result?: AuditResult }>();
  const recorder: AuditRecorder = {
    start(input) { const auditId = `run-${audits.size + 1}`; audits.set(auditId, { ...input, paths: [...input.paths] }); return auditId; },
    finish(auditId, result) { audits.get(auditId)!.result = result; },
    progress(auditId, result) { audits.get(auditId)!.result = { status: "completed", ...result }; },
    interrupt() {},
  };
  /** The steers the enforcer published (revision, sources they speak for, text), and the sources it revised. */
  const steers: Array<{ revision: number | undefined; scope: readonly string[]; content: string }> = [];
  const revisions: Array<{ revision: number; paths: readonly string[] }> = [];
  const enforcer = new NativeCodeRuleEnforcer(testCodeRuleCatalog(() => [current()], capabilitiesPath), {
    workspaceRoot, audits: recorder, ...(scope === undefined ? {} : { scope }),
    steerDelivery: {
      requestSteer: async (_session, _delivery, content, metadata) => {
        steers.push({ revision: metadata?.revision, scope: metadata?.scope ?? [], content });
        return undefined;
      },
      revised: (_session, revision, paths) => { revisions.push({ revision, paths: [...paths] }); },
    },
  });
  await enforcer.openWorkspace("package-facts");
  const ecmaPaths = Object.keys(sources).filter(path => ["javascript", "typescript", "tsx"].includes(languageFor(path) ?? ""));
  /** One log-only repository audit: violations per path and code, and the failures. */
  const audit = async () => {
    const [auditId] = await enforcer.audit("package-facts", { scope: "repository", mode: "log" });
    const result = audits.get(auditId!)?.result;
    return { status: result?.status, reported: counted(result?.violations ?? []), failures: result?.failures ?? [] };
  };
  /** The workspace's evaluation of `edits`, by default one that writes each ECMA file's source again. */
  let revision = 0;
  const edit = async (edits: CodeRuleEdit[] = ecmaPaths.map(path => ({ path, language: languageFor(path)!, generatedText: sources[path]! }))) => {
    const before = audits.size;
    revision += 1;
    await enforcer.auditGeneratedEdits("package-facts", { revision, checkpoints: ["edit"], edits });
    await enforcer.settle();
    const results = [...audits.values()].slice(before).map(entry => entry.result);
    expect(results.length).toBe(edits.length);
    expect(results.every(result => result?.status === "completed" && (result.failures ?? []).length === 0)).toBe(true);
    return counted(results.flatMap(result => result?.violations ?? []));
  };
  const compiled = loadNativeBinding().CodeRuleDslRuntime.compile(JSON.stringify(current()), "json", await readFile(capabilitiesPath, "utf8"));
  const evaluate = (files: ReadonlyArray<Record<string, unknown>>) => {
    const input = JSON.stringify({ format: "rerail.repository-snapshot/v1", revision: 0, facts: [], files });
    const result = JSON.parse(compiled.evaluate(input, "report")) as { violations: Array<{ path?: string; code: string }>; failures: unknown[] };
    expect(result.failures).toEqual([]);
    return counted(result.violations);
  };
  /** A stateless evaluation of every ECMA file with the facts the host sends. */
  const snapshot = async () =>
    evaluate(await Promise.all(ecmaPaths.map(async path => ({
      path,
      language: languageFor(path),
      source: sources[path]!,
      ...(await sourceFileFacts(workspaceRoot, path, languageFor(path)!, undefined, compiled.readsServerGraph())),
    }))));
  /** A stateless evaluation of the same files without facts, beside the package.json and tsconfig.json files on disk. */
  const derived = async () => {
    const manifests = await Promise.all(
      Object.keys(sources)
        .filter(path => /(?:^|\/)(?:package\.json|(?:ts|js)config[^/]*\.json)$/.test(path))
        .map(async path => ({ path, language: "json", source: await readFile(join(workspaceRoot, path), "utf8") })),
    );
    return evaluate([...manifests, ...ecmaPaths.map(path => ({ path, language: languageFor(path), source: sources[path]! }))]);
  };
  return {
    audit, edit, snapshot, derived, steers, revisions, enforcer,
    acceptsPath: (path: string) => enforcer.acceptsPath(path),
    close: () => enforcer.close(),
  };
}

const MODES = [
  ["the native repository index (Git checkout)", true],
  ["the host's directory walk", false],
] as const;

describe("a JavaScript or TypeScript file's module system", () => {
  /** `require` as a declared global: CommonJS files only. */
  const CODE_RULE = codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "source-type:require", revision: 1, mode: "steer",
    scope: { include: ["**/*"], exclude: [] },
    clauses: [{
      id: "require",
      violation: { code: "REQUIRE", anchor: "witness.span", message: "CommonJS require" },
      forbid: { ast: { language: "typescript", languageOptions: {}, rule: { kind: "identifier", resolves: { global: ["require"] } } } },
    }],
  });
  const SOURCES: Sources = {
    "package.json": "{}\n",
    "lib/plain.js": "module.exports = require('x');\n",
    "lib/esm.js": "import x from 'x';\nexport default require;\n",
    "lib/s.cjs": "exports.a = require('x');\n",
    "lib/m.mjs": "export const a = require;\n",
    "esm/package.json": '{ "type": "module" }\n',
    "esm/a.js": "module.exports = require('x');\n",
    // Node runs a .ts file as it runs a .js one, detecting after type stripping: a type-only import is no module syntax.
    "lib/plain.ts": "import type { A } from 'a';\nmodule.exports = require('x');\n",
    "lib/esm.ts": "import x from 'x';\nexport default require;\n",
    "esm/b.ts": "module.exports = require('x');\n",
    "lib/view.tsx": "export default require;\n",
  };
  /** The root manifest sets no `type`: `.js` and `.ts` files read their syntax. */
  const UNSET = { "lib/plain.js REQUIRE": 1, "lib/s.cjs REQUIRE": 1, "lib/plain.ts REQUIRE": 1 };
  /** The root manifest says `module`: only the `.cjs` file is CommonJS. */
  const MODULE = { "lib/s.cjs REQUIRE": 1 };

  it.each(MODES)("follows its package.json type alike through %s, an edit and a snapshot, and follows a type change", async (_name, git) => {
    const root = await repository(git, SOURCES);
    const { audit, edit, snapshot, derived, close } = await enforcerFor(root, CODE_RULE, SOURCES);
    try {
      expect((await sourceFileFacts(root, "lib/plain.js", "javascript")).package).toMatchObject({ folder: "", type: "unset" });
      expect((await sourceFileFacts(root, "esm/a.js", "javascript")).package).toMatchObject({ folder: "esm", type: "module" });
      expect(await audit()).toEqual({ status: "completed", reported: UNSET, failures: [] });
      expect(await edit()).toEqual(UNSET);
      expect(await snapshot()).toEqual(UNSET);
      expect(await derived()).toEqual(UNSET);

      // The same sources under a manifest that now says `module`: every path re-reads the files below it.
      await writeFile(join(root, "package.json"), '{ "type": "module" }\n');
      expect(await audit()).toEqual({ status: "completed", reported: MODULE, failures: [] });
      expect(await edit()).toEqual(MODULE);
      expect(await snapshot()).toEqual(MODULE);
      expect(await derived()).toEqual(MODULE);
    } finally {
      await close();
    }
  });
});

describe("file.package and file.jsxRuntime", () => {
  const program = (id: string, file: Record<string, unknown>) => ({
    id,
    violation: { code: id.toUpperCase(), anchor: "witness.span", message: id },
    forbid: { ast: { language: "typescript", file, rule: { kind: "program" } } },
  });
  const CODE_RULE = codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "package-facts:conditions", revision: 1, mode: "steer",
    scope: { include: ["**/*"], exclude: [] },
    clauses: [
      program("bin", { package: { bin: true } }),
      program("node", { package: { nodeVersion: { subsetOf: ">=20" } } }),
      program("next", { package: { dependsOn: ["next"] } }),
      program("react", { jsxRuntime: ["react"] }),
    ],
  });
  const SOURCES: Sources = {
    "package.json": '{ "bin": "./cli.js", "engines": { "node": ">=18" }, "dependencies": { "react": "19" } }\n',
    "cli.js": "#!/usr/bin/env node\nexport {};\n",
    "lib/a.js": "export {};\n",
    "web/package.json": '{ "dependencies": { "next": "15" }, "engines": { "node": ">=22" } }\n',
    "web/page.jsx": "export default () => <div />;\n",
    "web/preact.jsx": "/** @jsxImportSource preact */\nexport default () => <div />;\n",
    // React and Preact both listed: the project configuration's jsxImportSource decides, here from a base in a sibling
    // directory (JSONC, a comment after a trailing comma).
    "mixed/package.json": '{ "dependencies": { "react": "19", "preact": "10" } }\n',
    "mixed/tsconfig.json": '{\n  "extends": "../config/tsconfig.base.json",\n}\n',
    "config/tsconfig.base.json": '{\n  "compilerOptions": { "jsx": "react-jsx", "jsxImportSource": "preact", /* Preact */ },\n}\n',
    "mixed/view.jsx": "export default () => <div />;\n",
  };
  const BEFORE = {
    "cli.js BIN": 1, "cli.js REACT": 1,
    "lib/a.js REACT": 1,
    "web/page.jsx NODE": 1, "web/page.jsx NEXT": 1, "web/page.jsx REACT": 1,
    "web/preact.jsx NODE": 1, "web/preact.jsx NEXT": 1,
  };
  /** The root manifest now names `lib/a.js` its bin, needs Node 20 and depends on Vue. */
  const AFTER = {
    "cli.js NODE": 1,
    "lib/a.js BIN": 1, "lib/a.js NODE": 1,
    "web/page.jsx NODE": 1, "web/page.jsx NEXT": 1, "web/page.jsx REACT": 1,
    "web/preact.jsx NODE": 1, "web/preact.jsx NEXT": 1,
  };

  it.each(MODES)("read the nearest package.json alike through %s, an edit and a snapshot, and follow its edits", async (_name, git) => {
    const root = await repository(git, SOURCES);
    const { audit, edit, snapshot, derived, close } = await enforcerFor(root, CODE_RULE, SOURCES);
    try {
      expect((await sourceFileFacts(root, "web/page.jsx", "javascript")).package).toEqual({
        folder: "web", type: "unset", bin: [], nodeVersion: ">=22", dependencies: ["next"],
      });
      expect(await audit()).toEqual({ status: "completed", reported: BEFORE, failures: [] });
      expect(await edit()).toEqual(BEFORE);
      expect(await snapshot()).toEqual(BEFORE);
      expect(await derived()).toEqual(BEFORE);

      await writeFile(join(root, "package.json"), '{ "bin": { "tool": "lib/a" }, "engines": { "node": ">=20" }, "dependencies": { "vue": "3" } }\n');
      expect(await audit()).toEqual({ status: "completed", reported: AFTER, failures: [] });
      expect(await edit()).toEqual(AFTER);
      expect(await snapshot()).toEqual(AFTER);
      expect(await derived()).toEqual(AFTER);

      // The base configuration's edit alone, outside the package it configures, resends the source whose runtime it
      // changes.
      const emotion = '{ "compilerOptions": { "jsx": "react-jsx", "jsxImportSource": "@emotion/react" } }\n';
      await writeFile(join(root, "config/tsconfig.base.json"), emotion);
      expect(await edit([{ path: "config/tsconfig.base.json", language: "json", generatedText: emotion }])).toEqual({ "mixed/view.jsx REACT": 1 });
      const emotionRuntime = { ...AFTER, "mixed/view.jsx REACT": 1 };
      expect(await audit()).toEqual({ status: "completed", reported: emotionRuntime, failures: [] });
      expect(await snapshot()).toEqual(emotionRuntime);
      expect(await derived()).toEqual(emotionRuntime);
    } finally {
      await close();
    }
  });
});

describe("a package.json edit alone", () => {
  const program = (id: string, file?: Record<string, unknown>) => ({
    id,
    violation: { code: id.toUpperCase(), anchor: "witness.span", message: id },
    forbid: { ast: { language: "typescript", ...(file === undefined ? {} : { file }), rule: { kind: "program" } } },
  });
  /** JavaScript and TypeScript files only, so no code rule reads package.json itself. `PROGRAM` reports every evaluation of a file. */
  const CODE_RULE = codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "package-facts:manifest-edit", revision: 1, mode: "steer",
    scope: { include: ["**/*.js", "**/*.ts"], exclude: [] },
    clauses: [
      {
        id: "require",
        violation: { code: "REQUIRE", anchor: "witness.span", message: "CommonJS require" },
        forbid: { ast: { language: "typescript", languageOptions: {}, rule: { kind: "identifier", resolves: { global: ["require"] } } } },
      },
      program("program"),
      program("bin", { package: { bin: true } }),
      program("node", { package: { nodeVersion: { subsetOf: ">=20" } } }),
      program("next", { package: { dependsOn: ["next"] } }),
    ],
  });
  const SOURCES: Sources = {
    "package.json": '{ "type": "commonjs" }\n',
    "lib/plain.js": "module.exports = require('x');\n",
    // A .ts file reads its package as a .js one does; its type-only import is stripped before Node's detection.
    "lib/typed.ts": "import type { A } from 'a';\nmodule.exports = require('x');\n",
  };
  /** `codes` reported on both files, and `BIN` on the bin target, lib/plain.js. */
  const both = (codes: string[], bin = false) => Object.fromEntries([
    ...codes.flatMap(code => [[`lib/plain.js ${code}`, 1], [`lib/typed.ts ${code}`, 1]]),
    ...(bin ? [["lib/plain.js BIN", 1]] : []),
  ]);

  it("refreshes the package of the sources applied below it, created, changed or deleted, without resending them", async () => {
    const root = await repository(false, SOURCES);
    const { edit, snapshot, acceptsPath, close } = await enforcerFor(root, CODE_RULE, SOURCES);
    /** Writes (or, with `undefined`, deletes) package.json and sends that edit alone: what its run reports. */
    const manifest = async (text: string | undefined) => {
      if (text === undefined) await unlink(join(root, "package.json"));
      else await writeFile(join(root, "package.json"), text);
      return edit([text === undefined
        ? { path: "package.json", language: "json", deleted: true }
        : { path: "package.json", language: "json", generatedText: text }]);
    };
    try {
      // Routed whatever the code rules' scopes, as are project configurations; another file outside them still is not.
      expect(acceptsPath("package.json")).toBe(true);
      expect(acceptsPath("lib/package.json")).toBe(true);
      expect(acceptsPath("tsconfig.json")).toBe(true);
      expect(acceptsPath("data/settings.json")).toBe(false);
      const steps: Array<[string | undefined, Record<string, number>]> = [
        ['{ "type": "module" }\n', both(["PROGRAM"])],
        ['{ "type": "module", "engines": { "node": ">=22" } }\n', both(["PROGRAM", "NODE"])],
        ['{ "type": "module", "engines": { "node": ">=22" }, "bin": "./lib/plain.js", "dependencies": { "next": "15" } }\n', both(["PROGRAM", "NODE", "NEXT"], true)],
        ['{ "type": "commonjs" }\n', both(["PROGRAM", "REQUIRE"])],
        // No `type`: Node's syntax detection reads both as CommonJS.
        ["{}\n", both(["PROGRAM", "REQUIRE"])],
        // Without a manifest a .js or .ts file is a module.
        [undefined, both(["PROGRAM"])],
        // eslint-plugin-n drops `.js` alone when it matches a bin target, so lib/typed.ts is none.
        ['{ "type": "commonjs", "bin": { "tool": "lib/plain" } }\n', both(["PROGRAM", "REQUIRE"], true)],
      ];
      expect(await edit()).toEqual(both(["PROGRAM", "REQUIRE"]));
      for (const [text, reported] of steps) {
        expect(await manifest(text), text ?? "deleted").toEqual(reported);
        // As a fresh evaluation with the facts on disk reads it.
        expect(await snapshot(), text ?? "deleted").toEqual(reported);
      }
      // A manifest edit that changes no source's package evaluates none of them again.
      expect(await manifest('{ "name": "renamed", "type": "commonjs", "bin": { "tool": "lib/plain" } }\n')).toEqual({});
      // The sources themselves, sent again, read the current manifest.
      expect(await edit()).toEqual(both(["PROGRAM", "REQUIRE"], true));
    } finally {
      await close();
    }
  });

  it("lets a source's own later edit in the same batch replace the findings a manifest edit gave it", async () => {
    const sources: Sources = { "package.json": '{ "type": "module" }\n', "lib/plain.js": "module.exports = require('x');\n" };
    const root = await repository(false, sources);
    const { edit, steers, revisions, close } = await enforcerFor(root, CODE_RULE, sources);
    try {
      expect(await edit([{ path: "lib/plain.js", language: "javascript", generatedText: sources["lib/plain.js"]! }])).toEqual({
        "lib/plain.js PROGRAM": 1,
      });
      // One batch: the manifest turns CommonJS, which gives the applied source a require finding, then the source drops
      // its require. The steer published for the batch speaks for the source as it ends.
      await writeFile(join(root, "package.json"), '{ "type": "commonjs" }\n');
      await writeFile(join(root, "lib/plain.js"), "export {};\n");
      steers.length = 0;
      await edit([
        { path: "package.json", language: "json", generatedText: '{ "type": "commonjs" }\n' },
        { path: "lib/plain.js", language: "javascript", generatedText: "export {};\n" },
      ]);
      expect(steers.length).toBeGreaterThan(0);
      expect(steers.map(steer => steer.content).join("\n")).toContain("program");
      expect(steers.map(steer => steer.content).join("\n")).not.toContain("CommonJS require");
      expect(steers.flatMap(steer => steer.scope)).toContain("lib/plain.js");
      // The manifest's refresh of the source is a revision of it, so an earlier steer for it is stale.
      expect(revisions).toContainEqual({ revision: 2, paths: ["lib/plain.js"] });
    } finally {
      await close();
    }
  });

  it("sends a source again when the manifest edit that refreshed it is superseded before it publishes", async () => {
    const sources: Sources = { "package.json": '{ "type": "commonjs" }\n', "lib/plain.js": "module.exports = require('x');\n" };
    const root = await repository(false, sources);
    const { edit, revisions, enforcer, close } = await enforcerFor(root, CODE_RULE, sources);
    const prototype = loadNativeBinding().CodeRuleDslWorkspace.prototype;
    const applyBufferAsync = prototype.applyBufferAsync;
    try {
      expect(await edit([{ path: "lib/plain.js", language: "javascript", generatedText: sources["lib/plain.js"]! }])).toEqual({
        "lib/plain.js PROGRAM": 1,
        "lib/plain.js REQUIRE": 1,
      });
      // Revision 2 makes the package a module, which refreshes the source; revision 3 edits the manifest again while
      // revision 2's request is in flight, so revision 2 never publishes. Revision 3 leaves the facts as 2 set them.
      await writeFile(join(root, "package.json"), '{ "type": "module" }\n');
      let queued: Promise<unknown> | undefined;
      const spy = vi.spyOn(prototype, "applyBufferAsync").mockImplementation(function (this: unknown, ...args: unknown[]) {
        queued ??= enforcer.auditGeneratedEdits("package-facts", {
          revision: 3, checkpoints: ["edit"],
          edits: [{ path: "package.json", language: "json", generatedText: '{ "type": "module" }\n' }],
        });
        return Reflect.apply(applyBufferAsync, this, args);
      });
      await enforcer.auditGeneratedEdits("package-facts", {
        revision: 2, checkpoints: ["edit"],
        edits: [{ path: "package.json", language: "json", generatedText: '{ "type": "module" }\n' }],
      });
      await enforcer.settle();
      await queued;
      spy.mockRestore();
      await enforcer.settle();
      // The superseded revision recorded nothing it sent, so revision 3 sends the source again and reports it.
      expect(revisions).toContainEqual({ revision: 3, paths: ["lib/plain.js"] });
    } finally {
      vi.restoreAllMocks();
      await close();
    }
  });

  it("in ai-output scope leaves the findings and steers of the sources it repackages to their own edits", async () => {
    const sources: Sources = { "package.json": '{ "type": "commonjs" }\n', "lib/plain.js": "module.exports = require('x');\n" };
    const root = await repository(false, sources);
    const { edit, steers, revisions, close } = await enforcerFor(root, CODE_RULE, sources, "ai-output");
    try {
      expect(await edit([{ path: "lib/plain.js", language: "javascript", generatedText: sources["lib/plain.js"]! }])).toEqual({
        "lib/plain.js PROGRAM": 1,
        "lib/plain.js REQUIRE": 1,
      });
      expect(steers.map(steer => [steer.revision, steer.scope, steer.content.includes("CommonJS require")])).toEqual([
        [1, ["lib/plain.js"], true],
      ]);
      // A dependency the rules ignore gives the source another `package`, so the manifest edit sends it again. Its
      // generated lines are the manifest's, which report none of the source's findings: the source's steer stands.
      const manifest = '{ "type": "commonjs", "dependencies": { "left-pad": "1" } }\n';
      await writeFile(join(root, "package.json"), manifest);
      expect(await edit([{ path: "package.json", language: "json", generatedText: manifest }])).toEqual({});
      expect(revisions).toEqual([]);
      expect(steers).toHaveLength(1);
    } finally {
      await close();
    }
  });
});

describe("a Next app's server module graph (file.package.serverGraph)", () => {
  const program = (id: string, serverGraph: boolean) => ({
    id,
    violation: { code: id.toUpperCase(), anchor: "witness.span", message: id },
    forbid: { ast: { language: "typescript", file: { package: { serverGraph } }, rule: { kind: "program" } } },
  });
  const CODE_RULE = codeRuleSchema.parse({
    format: "rerail.policy/v1", id: "package-facts:server-graph", revision: 1, mode: "steer",
    scope: { include: ["**/*.ts", "**/*.tsx"], exclude: [] },
    clauses: [program("server", true), program("client", false)],
  });
  const CLIENT_WIDGET = "'use client';\nimport { useCounter } from './adapters/react';\nexport default function Widget() { return useCounter(); }\n";
  const sources = (): Sources => ({
    "package.json": '{ "dependencies": { "next": "16.0.0" } }\n',
    "tsconfig.json": '{\n  // the default alias\n  "compilerOptions": { "paths": { "@/*": ["./*"] } },\n}\n',
    "app/page.tsx": "import Widget from './widget';\nimport { useToggle } from '@/lib/toggle';\nimport './bridge';\nexport default function Page() { useToggle(); return <Widget />; }\n",
    "app/bridge/index.ts": "export * from '../shared/hook';\n",
    "app/shared/hook.ts": "export const hook = 1;\n",
    "app/widget.tsx": CLIENT_WIDGET,
    "app/adapters/react.ts": "import { useState } from 'react';\nexport const useCounter = () => useState(0)[0];\n",
    "lib/toggle.ts": "import { useReducer } from 'react';\nexport const useToggle = () => useReducer((on: boolean) => !on, false)[0];\n",
  });
  /** zechub: the module only a "use client" module imports is client code; the aliased one is the page's. */
  const ZECHUB = {
    "app/page.tsx SERVER": 1,
    "app/widget.tsx CLIENT": 1,
    "app/adapters/react.ts CLIENT": 1,
    "lib/toggle.ts SERVER": 1,
    "app/bridge/index.ts SERVER": 1,
    "app/shared/hook.ts SERVER": 1,
  };
  /** The widget without its directive: it and the module it imports join the server graph. */
  const SERVER = {
    "app/page.tsx SERVER": 1,
    "app/widget.tsx SERVER": 1,
    "app/adapters/react.ts SERVER": 1,
    "lib/toggle.ts SERVER": 1,
    "app/bridge/index.ts SERVER": 1,
    "app/shared/hook.ts SERVER": 1,
  };

  it.each(MODES)("is read alike through %s, an edit and a snapshot, and an edit that changes it resends the sources it moves", async (_name, git) => {
    const files = sources();
    const root = await repository(git, files);
    const { audit, edit, snapshot, derived, close } = await enforcerFor(root, CODE_RULE, files);
    try {
      expect(await audit()).toEqual({ status: "completed", reported: ZECHUB, failures: [] });
      expect(await edit()).toEqual(ZECHUB);
      expect(await snapshot()).toEqual(ZECHUB);
      expect(await derived()).toEqual(ZECHUB);

      // The widget's edit alone resends the module it imports, which was applied before.
      files["app/widget.tsx"] = CLIENT_WIDGET.replace("'use client';\n", "");
      await writeFile(join(root, "app/widget.tsx"), files["app/widget.tsx"]);
      expect(await edit([{ path: "app/widget.tsx", language: "tsx", generatedText: files["app/widget.tsx"] }])).toEqual({
        "app/widget.tsx SERVER": 1,
        "app/adapters/react.ts SERVER": 1,
      });
      expect(await audit()).toEqual({ status: "completed", reported: SERVER, failures: [] });
      expect(await snapshot()).toEqual(SERVER);
      expect(await derived()).toEqual(SERVER);

      // A nested package around the bridge is no edge: the hook beyond it, outside the new package, leaves the graph, and
      // the manifest's edit alone resends it.
      files["app/bridge/package.json"] = "{}\n";
      await writeFile(join(root, "app/bridge/package.json"), "{}\n");
      expect(await edit([{ path: "app/bridge/package.json", language: "json", generatedText: "{}\n" }])).toEqual({
        "app/shared/hook.ts CLIENT": 1,
      });
      const { "app/bridge/index.ts SERVER": _bridge, "app/shared/hook.ts SERVER": _hook, ...rest } = SERVER;
      const nested = { ...rest, "app/shared/hook.ts CLIENT": 1 };
      expect(await audit()).toEqual({ status: "completed", reported: nested, failures: [] });
      expect(await snapshot()).toEqual(nested);
      expect(await derived()).toEqual(nested);
    } finally {
      await close();
    }
  });

  it("keeps each source's membership through a catalog replacement, which reads each package's graph once", async () => {
    const files = sources();
    const root = await repository(false, files);
    let codeRule = CODE_RULE;
    const { edit, close } = await enforcerFor(root, () => codeRule, files);
    const walks = vi.spyOn(loadNativeBinding(), "serverGraphAsync");
    try {
      expect(await edit()).toEqual(ZECHUB);
      // A new revision replaces the workspace before the next edit: every applied source is applied again with facts
      // from one walk of the graph on disk, then the edit walks it once with its own text.
      codeRule = codeRuleSchema.parse({ ...CODE_RULE, revision: 2, clauses: [program("server_v2", true), program("client_v2", false)] });
      walks.mockClear();
      expect(await edit([{ path: "lib/toggle.ts", language: "typescript", generatedText: files["lib/toggle.ts"]! }])).toEqual({
        "lib/toggle.ts SERVER_V2": 1,
      });
      expect(walks.mock.calls.map(([, folder, overlayPath]) => [folder, overlayPath ?? null])).toEqual([["", null], ["", "lib/toggle.ts"]]);
      // The replacement sent the widget's import as client code, so the widget's edit that drops its directive resends it.
      files["app/widget.tsx"] = CLIENT_WIDGET.replace("'use client';\n", "");
      await writeFile(join(root, "app/widget.tsx"), files["app/widget.tsx"]);
      expect(await edit([{ path: "app/widget.tsx", language: "tsx", generatedText: files["app/widget.tsx"] }])).toEqual({
        "app/widget.tsx SERVER_V2": 1,
        "app/adapters/react.ts SERVER_V2": 1,
      });
    } finally {
      walks.mockRestore();
      await close();
    }
  });

  it("routes a source outside the code rule's scope whose imports move a source inside it", async () => {
    // The rule reads only app/shared; the page that reaches the hook there is outside its scope.
    const scoped = codeRuleSchema.parse({ ...CODE_RULE, id: "package-facts:server-graph-scoped", scope: { include: ["app/shared/**"], exclude: [] } });
    const files = sources();
    const root = await repository(false, files);
    const { edit, audit, acceptsPath, close } = await enforcerFor(root, scoped, files);
    try {
      // Graph inputs route whatever the scope: sources, and files whose folder may hide src/app.
      expect(acceptsPath("app/page.tsx")).toBe(true);
      expect(acceptsPath("app/styles.json")).toBe(true);
      expect(acceptsPath("data/settings.json")).toBe(false);
      expect(await edit([{ path: "app/shared/hook.ts", language: "typescript", generatedText: files["app/shared/hook.ts"]! }])).toEqual({
        "app/shared/hook.ts SERVER": 1,
      });
      // The page drops its import of the bridge: its edit is routed, and resends the hook it no longer reaches.
      files["app/page.tsx"] = files["app/page.tsx"]!.replace("import './bridge';\n", "");
      await writeFile(join(root, "app/page.tsx"), files["app/page.tsx"]);
      expect(await edit([{ path: "app/page.tsx", language: "tsx", generatedText: files["app/page.tsx"] }])).toEqual({
        "app/shared/hook.ts CLIENT": 1,
      });
      expect(await audit()).toEqual({ status: "completed", reported: { "app/shared/hook.ts CLIENT": 1 }, failures: [] });
    } finally {
      await close();
    }
  });
});
