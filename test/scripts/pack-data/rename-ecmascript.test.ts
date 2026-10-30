import { describe, expect, it } from "vitest";

import { renameSource, renameVariant, syntaxDiagnostics } from "../../../scripts/pack-data/rename/ecmascript.js";

const sign = (value: number): number => Math.sign(value);
const comparisons: Array<[string, (left: string, right: string) => number]> = [
  ["code units", (left, right) => (left < right ? -1 : left > right ? 1 : 0)],
  ["ignoring case", (left, right) => (left.toLowerCase() < right.toLowerCase() ? -1 : left.toLowerCase() > right.toLowerCase() ? 1 : 0)],
  ["locale", (left, right) => left.localeCompare(right)],
];

describe("renameSource", () => {
  it("renames identifiers through one same-shape map, keeping the length", () => {
    const source = "const foo = { bar: 1 };\nif (foo.bar) console.log(foo);\nclass Widget { #secret = 1; }\n";
    const { source: renamed, renames } = renameSource(source, "a.js");
    expect(renamed).toHaveLength(source.length);
    expect(renames.get("bar")).toMatch(/^[a-z]{3}$/);
    expect(renamed).toContain(`${renames.get("foo")}.${renames.get("bar")}`);
    expect(renames.get("Widget")).toMatch(/^[A-Z][a-z]{5}$/);
    expect(renames.get("#secret")).toMatch(/^#[a-z]{6}$/);
    for (const kept of ["const", "console", "log", "class"]) expect(renames.has(kept)).toBe(false);
  });

  it("keeps every literal, and the names that are words in one", () => {
    const source =
      '"use strict";\nimport fs from "node:fs";\nconst r = require("./mod");\nconst n = [0, 10, 0x1F, 2.5, 1e3];\nconst cfg = { key: parseInt("071", 10), "key": 2, other: `font ${fs} Inter` };\nif (cfg.hasOwnProperty("other")) location.href = "javascript:void(0)";\n';
    const { source: renamed, renames } = renameSource(source, "a.js");
    for (const kept of ['"use strict"', '"node:fs"', 'require("./mod")', "[0, 10, 0x1F, 2.5, 1e3]", 'parseInt("071", 10)', '"key": 2', "`font ${", "} Inter`", '"javascript:void(0)"', 'hasOwnProperty("other")']) {
      expect(renamed).toContain(kept);
    }
    expect(renames.has("key")).toBe(false);
    expect(renames.has("other")).toBe(false);
    expect(renames.has("cfg")).toBe(true);
  });

  it("keeps every two names' order: by code units, ignoring case and by locale", () => {
    const names = ["a", "b", "B", "Ab", "aB", "abc", "abd", "Abe", "zeta", "Zeta", "alpha", "alphaB", "x1", "x10", "x2", "_y", "$z", "mixedCase", "MixedCase", "mixedcase", "q"];
    const source = `var ${names.join(", ")};\nconst o = { "middle": 1, log: 2 };\n`;
    const { renames } = renameSource(source, "a.js");
    const all = [...names, "middle", "log"];
    const image = (name: string): string => renames.get(name) ?? name;
    expect(renames.size).toBeGreaterThan(names.length / 2);
    for (const [label, compare] of comparisons) {
      for (const left of all) for (const right of all) expect(sign(compare(image(left), image(right))), `${label}: ${left} vs ${right}`).toBe(sign(compare(left, right)));
    }
  });

  it("keeps protected names: test-runner globals and the members chained on them, hooks, and the standard library", () => {
    const { renames } = renameSource("vi.fn(); jest.mock(); it.only.each(); test.skip(); it.todo(); expect(x).not.toBe(1); useThing(); useState(); Math.max(); helper();\n", "a.ts");
    expect([...renames.keys()].sort()).toEqual(["helper", "x"]);
    expect(renames.get("helper")).not.toMatch(/^use[A-Z]/);
  });

  it("keeps what a package or a host object exports: imported names, members, component props, Sync names", () => {
    const source = [
      'import { Head } from "next/document";',
      'import Script from "next/script";',
      'import local from "./local";',
      "const { TextDecoder } = require('util');",
      "const fs = require('fs');",
      "fs.access(); fs.fooSync(); process.exit(0); ReactDOM.findDOMNode(inst); local.thing();",
      'const el = <Script strategy="lazy" data={local} />;',
      "class Widget extends React.Component { componentDidMount() { callback(err); } }",
      "",
    ].join("\n");
    const { renames } = renameSource(source, "a.jsx");
    for (const kept of ["Head", "TextDecoder", "access", "fooSync", "exit", "findDOMNode", "strategy", "data", "componentDidMount", "callback", "err"]) expect(renames.has(kept), kept).toBe(false);
    for (const renamed of ["thing", "inst", "Widget", "el"]) expect(renames.has(renamed), renamed).toBe(true);
  });

  it("renames the names a global comment lists with their uses", () => {
    const source = "/* global fooBar, bazQux: writable -- set by the page */\nfooBar(); bazQux = 1;\n// fooBar stays here\n";
    const { source: renamed, renames } = renameSource(source, "a.js");
    const fooBar = renames.get("fooBar")!;
    const bazQux = renames.get("bazQux")!;
    expect(renamed.split("\n")[0]).toBe(`/* global ${fooBar}, ${bazQux}: writable -- set by the page */`);
    expect(renamed).toContain(`${fooBar}(); ${bazQux} = 1;`);
    expect(renamed).toContain("// fooBar stays here");
  });

  it("keeps intrinsic JSX tags, their attributes and namespaced names, and renames user components", () => {
    const { source: renamed, renames } = renameSource('const a = <div className="x"><svg xlink:href="#a" /><Widget flavor={1} /></div>;\n', "a.jsx");
    expect(renamed).toContain('<div className="x"><svg xlink:href="#a" />');
    expect(renames.has("xlink")).toBe(false);
    expect(renames.has("Widget")).toBe(true);
    expect(renames.has("flavor")).toBe(true);
  });

  it("reads JSX in a .js file as JSX: a user component's closing tag is renamed with its opening tag", () => {
    const source = 'import Head from "next/document";\nexport const Page = () => <Head><link href={url} rel="icon" /></Head>;\n';
    const { source: renamed, renames } = renameSource(source, "page.js");
    const head = renames.get("Head")!;
    expect(head).toMatch(/^[A-Z][a-z]{3}$/);
    expect(renamed).toContain(`<${head}><link href={`);
    expect(renamed).toContain(`</${head}>`);
  });

  it("keeps meta-property names", () => {
    const { source: renamed, renames } = renameSource("const here = new URL(name, import.meta.url);\nfunction F() { return new.target; }\n", "a.mjs");
    expect(renamed).toContain("import.meta.");
    expect(renamed).toContain("new.target");
    expect(renames.has("meta")).toBe(false);
    expect(renames.has("target")).toBe(false);
  });
});

describe("renameVariant", () => {
  it("writes no variant when nothing would be renamed", () => {
    expect(renameVariant('console.log(0, "text");\n', "a.js")).toEqual({ problem: "nothing to rename" });
  });

  it("writes a variant of a .js file with JSX that parses as the original does", () => {
    const source = 'import Head from "next/head";\nexport default function Page() {\n  return <Head><title>{name}</title></Head>;\n}\n';
    const variant = renameVariant(source, "pages/index.js");
    if (!("source" in variant)) throw new Error(`no variant: ${variant.problem}`);
    expect(syntaxDiagnostics(variant.source, "pages/index.js")).toEqual([]);
  });
});
