/**
 * react-hooks-rules-of-hooks must not take a `use` that the file binds to something other than React for React's `use`
 * hook (#250).
 *
 * eslint-plugin-react-hooks 7.1.1 names a hook by its callee alone (isHook: `use`, `useX`, or `Namespace.useX` with a
 * PascalCase namespace), so chai's `use(plugin)` and markdown-it's `Md.use(plugin)` at the top level of a test or
 * module are reported as hooks called outside a component. The port departs from upstream for the exact name `use`
 * only: a `use` callee, or the namespace of an `X.use` callee, that the file binds to anything but React (React being
 * an import from `react` or a declarator initialised from `React` or `require('react')`) is not a hook. An unbound
 * `use` / `X.use`, and every `useX` name, stay hooks as upstream has them.
 */
import { readFile } from "node:fs/promises";
import { join } from "node:path";
import { fileURLToPath } from "node:url";
import { z } from "zod";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../../../../../src/engine/native/native.js";
import { repositoryPath } from "../../../../support/oracles.js";

const CODE_RULE_DIRECTORY = fileURLToPath(new URL("../../../../../native/src/code_rule_dsl/", import.meta.url));
const capabilities = await readFile(join(CODE_RULE_DIRECTORY, "capabilities.json"), "utf8");
const runtime = loadNativeBinding().CodeRuleDslRuntime;
const RULE_ID = "react-hooks-rules-of-hooks";
const document = await readFile(repositoryPath("packs", "react", "rules", `${RULE_ID}.json`), "utf8");

const evaluationSchema = z.object({
  violations: z
    .array(z.object({ span: z.object({ startLine: z.number().int().nonnegative(), startColumn: z.number().int().nonnegative() }) }))
    .default([]),
  failures: z.array(z.unknown()).default([]),
});

if (runtime === undefined) throw new Error("Native binding must export CodeRuleDslRuntime");
const rule = runtime.compile(document, "json", capabilities);

/** The sorted 1-based line:column positions the rule reports in `source`, as ESLint prints them. */
function reported(source: string, path = "src/setup.ts"): string[] {
  const language = path.endsWith(".ts") || path.endsWith(".tsx") ? "typescript" : "javascript";
  const input = JSON.stringify({
    format: "rerail.repository-snapshot/v1",
    revision: 0,
    facts: [],
    files: [{ language, path, source }],
  });
  const result = evaluationSchema.parse(JSON.parse(rule.evaluate(input, "report")));
  expect(result.failures).toEqual([]);
  return result.violations
    .map(violation => [violation.span.startLine + 1, violation.span.startColumn + 1] as const)
    .sort((a, b) => a[0] - b[0] || a[1] - b[1])
    .map(([line, column]) => `${line}:${column}`);
}

const lines = (...source: string[]) => [...source, ""].join("\n");

describe(RULE_ID, () => {
  it("does not report chai's imported use or markdown-it's Md.use at the top level (issue repro)", () => {
    const source = lines(
      "import { expect, use } from 'chai';",
      "use(chaiAsPromised);",
      "",
      "const Md = markdownIt();",
      "Md.use(MdTaskList);",
    );
    expect(reported(source)).toEqual([]);
    expect(reported(source, "test/setup.js")).toEqual([]);
  });

  it("does not report a non-React use under any import form, a local use, or one in a function or try block", () => {
    const source = lines(
      "import chai from 'chai';",
      "import * as Chai from 'chai';",
      "import I18n from 'i18next';",
      "import { plugin as use } from './plugins';",
      "Chai.use(chaiAsPromised);",
      "I18n.use(initReactI18next);",
      "function setup() { use(plugin); }",
      "try { use(plugin); } catch {}",
      "class Server { start() { use(router); } }",
      "function Page() { const App = createApp(); if (ready) App.use(router); return null; }",
    );
    expect(reported(source)).toEqual([]);
    expect(reported(lines("function use(plugin: unknown) {}", "use(plugin);"))).toEqual([]);
    expect(reported(lines("const Md = markdownIt();", "export function render() { Md.use(MdTaskList); }"))).toEqual([]);
    expect(reported(lines("function setup(use: (p: unknown) => void) { use(plugin); }"))).toEqual([]);
  });

  it("still reports React's use imported from react, under an alias, through React, or unbound", () => {
    const source = lines(
      "import { use } from 'react';",
      "import * as React from 'react';",
      "use(promise);",
      "React.use(promise);",
      "function load() { use(promise); }",
      "function Page() { try { use(promise); } catch {} return null; }",
    );
    expect(reported(source)).toEqual(["3:1", "4:1", "5:19", "6:25"]);
    expect(reported(lines("import React from 'react';", "React.use(promise);"))).toEqual(["2:1"]);
    expect(reported(lines("import * as R from 'react';", "R.use(promise);"))).toEqual(["2:1"]);
    expect(reported(lines("use(promise);", "React.use(promise);", "Ns.use(promise);"))).toEqual(["1:1", "2:1", "3:1"]);
    expect(reported(lines("import * as React from 'react';", "const { use } = React;", "use(promise);"))).toEqual(["3:1"]);
    const required = lines("const { use } = require('react');", "const Re = require('react');", "use(promise);", "Re.use(promise);");
    expect(reported(required, "src/load.js")).toEqual(["3:1", "4:1"]);
  });

  it("still reports every useX hook, whatever binds it", () => {
    const source = lines(
      "import { useStore } from 'zustand';",
      "import Store from './store';",
      "useStore();",
      "Store.useState();",
      "function load() { useStore(); }",
    );
    expect(reported(source)).toEqual(["3:1", "4:1", "5:19"]);
  });

});
