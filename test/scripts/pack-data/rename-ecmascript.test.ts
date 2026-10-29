import { describe, expect, it } from "vitest";

import { renameSource, renameVariant, syntaxDiagnostics } from "../../../scripts/pack-data/rename/ecmascript.js";

describe("renameSource", () => {
  it("renames identifiers and literal words through one same-shape map, keeping the length", () => {
    const source = 'const foo = { bar: 1 };\nif (foo.hasOwnProperty("bar")) console.log("Hello world");\nconst t = `of ${foo.bar} baz`;\n';
    const { source: renamed, renames } = renameSource(source, "a.js");
    expect(renamed).toHaveLength(source.length);
    const bar = renames.get("bar")!;
    expect(bar).toMatch(/^[a-z]{3}$/);
    expect(renamed).toContain(`hasOwnProperty("${bar}")`);
    expect(renamed).toContain(`${renames.get("foo")}.${bar}`);
    expect(renames.get("Hello")).toMatch(/^[A-Z][a-z]{4}$/);
    expect(renames.has("baz")).toBe(true);
    for (const kept of ["const", "console", "log", "hasOwnProperty", "of"]) expect(renames.has(kept)).toBe(false);
  });

  it("keeps module specifiers, directives, escapes and protected words, and permutes digits keeping zero", () => {
    const source = '"use strict";\nimport fs from "node:fs";\nconst r = require("./mod");\nconst s = "\\u0061\\x41" + typeof r === "function";\nconst n = [0, 10, 10, 2.5, 0o17, 0b11, 1e3];\n';
    const { source: renamed } = renameSource(source, "a.js");
    for (const kept of ['"use strict"', '"node:fs"', 'require("./mod")', '"\\u0061\\x41"', '"function"', "0o17", "0b11", "e3"]) expect(renamed).toContain(kept);
    const numbers = /\[(.*)\]/.exec(renamed.split("\n")[4]!)![1]!.split(", ");
    expect(numbers[0]).toBe("0");
    expect(numbers[1]).toMatch(/^[1-9]0$/);
    expect(numbers[2]).toBe(numbers[1]);
  });

  it("keeps intrinsic JSX tags and their attributes and renames user components", () => {
    const { source: renamed, renames } = renameSource('const a = <div className="x"><Widget flavor={1} /></div>;\n', "a.jsx");
    expect(renamed).toContain("<div className=");
    expect(renames.has("Widget")).toBe(true);
    expect(renames.has("flavor")).toBe(true);
  });

  it("reads JSX in a .js file as JSX: a user component's closing tag is renamed with its opening tag", () => {
    const source = 'import Head from "next/head";\nexport const Page = () => <Head><link href={url} rel="icon" /></Head>;\n';
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
    expect(renameVariant("console.log(0);\n", "a.js")).toEqual({ problem: "nothing to rename" });
  });

  it("writes a variant of a .js file with JSX that parses as the original does", () => {
    const source = 'import Head from "next/head";\nexport default function Page() {\n  return <Head><title>{name}</title></Head>;\n}\n';
    const variant = renameVariant(source, "pages/index.js");
    if (!("source" in variant)) throw new Error(`no variant: ${variant.problem}`);
    expect(syntaxDiagnostics(variant.source, "pages/index.js")).toEqual([]);
  });
});
