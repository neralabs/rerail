// The layer boundary lint (scripts/check/check-boundaries.mjs, metric #13): each rule passes and fails on
// a tiny checkout written to a temporary directory.

import { mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { checkBoundaries } from "../../scripts/check/check-boundaries.mjs";

const roots = [];

afterEach(() => {
  for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true });
});

/** The violations, as `file:line [rule]`, of a checkout holding `tree` (path → contents). */
function violations(tree) {
  const root = mkdtempSync(join(tmpdir(), "rerail-boundaries-"));
  roots.push(root);
  for (const [path, text] of Object.entries(tree)) {
    mkdirSync(dirname(join(root, path)), { recursive: true });
    writeFileSync(join(root, path), text);
  }
  return checkBoundaries(root, { files: Object.keys(tree).sort() }).violations.map(
    (violation) => `${violation.file}:${violation.line} [${violation.rule}]`,
  );
}

describe("engine-isolation", () => {
  it("lets the engine import its own modules", () => {
    expect(violations({ "src/engine/a.ts": 'import { b } from "./b.js";', "src/engine/b.ts": "" })).toEqual([]);
  });

  it("flags the engine importing another layer", () => {
    expect(
      violations({ "src/engine/a.ts": 'import { b } from "./b.js";\nimport { r } from "../runtime/r.js";', "src/engine/b.ts": "", "src/runtime/r.ts": "" }),
    ).toEqual(["src/engine/a.ts:2 [engine-isolation]"]);
  });
});

describe("host-isolation", () => {
  it("lets a host import the engine and its own modules", () => {
    expect(
      violations({
        "src/hosts/omp/a.ts": 'import { e } from "../../engine/index.js";\nimport { b } from "./b.js";',
        "src/hosts/omp/b.ts": "",
        "src/engine/index.ts": "",
      }),
    ).toEqual([]);
  });

  it("flags a host importing another host", () => {
    expect(violations({ "src/hosts/omp/a.ts": 'import { p } from "../pi/p.js";', "src/hosts/pi/p.ts": "" })).toEqual([
      "src/hosts/omp/a.ts:1 [host-isolation]",
    ]);
  });
});

describe("layer-order", () => {
  it("lets a layer import the layers listed for it", () => {
    expect(violations({ "src/api/a.ts": 'import { r } from "../runtime/r.js";', "src/runtime/r.ts": "" })).toEqual([]);
  });

  it("flags a layer importing one not listed for it, and an undeclared layer", () => {
    expect(
      violations({
        "src/runtime/a.ts": 'import { x } from "../api/x.js";',
        "src/api/x.ts": "",
        "src/extra/a.ts": 'import { r } from "../runtime/r.js";',
        "src/runtime/r.ts": "",
      }),
    ).toEqual(["src/extra/a.ts:1 [layer-order]", "src/runtime/a.ts:1 [layer-order]"]);
  });
});

describe("src-self-contained", () => {
  it("lets src/ import packages and node built-ins", () => {
    expect(violations({ "src/api/a.ts": 'import { readFileSync } from "node:fs";\nimport ts from "typescript";' })).toEqual([]);
  });

  it("flags src/ importing a path outside src/", () => {
    expect(violations({ "src/api/a.ts": 'import pkg from "../../package.json" with { type: "json" };' })).toEqual([
      "src/api/a.ts:1 [src-self-contained]",
    ]);
  });
});

describe("entry-points", () => {
  it("lets code outside src/ import a layer's index.ts or src/index.ts", () => {
    expect(
      violations({
        "integration/a.test.ts": 'import { e } from "../src/engine/index.js";\nimport { p } from "../src/index.js";',
        "scripts/b.mjs": 'import { h } from "../src/hosts/omp/index.js";',
        "src/engine/index.ts": "",
        "src/hosts/omp/index.ts": "",
        "src/index.ts": "",
      }),
    ).toEqual([]);
  });

  it("flags code outside src/ importing a layer's internals", () => {
    expect(
      violations({ "integration/a.test.ts": 'import { b } from "../src/engine/b.js";', "src/engine/b.ts": "" }),
    ).toEqual(["integration/a.test.ts:1 [entry-points]"]);
  });

  it("lets unit tests import their own layer's internals, not another layer's", () => {
    expect(
      violations({
        "test/engine/b.test.ts": 'import { b } from "../../src/engine/b.js";\nimport { r } from "../../src/runtime/r.js";',
        "test/hosts/omp/h.test.ts": 'import { h } from "../../../src/hosts/omp/h.js";\nimport { p } from "../../../src/hosts/pi/p.js";',
        "src/engine/b.ts": "",
        "src/runtime/r.ts": "",
        "src/hosts/omp/h.ts": "",
        "src/hosts/pi/p.ts": "",
      }),
    ).toEqual(["test/engine/b.test.ts:2 [entry-points]", "test/hosts/omp/h.test.ts:2 [entry-points]"]);
  });

  it("lets a vitest mock seam name the module it replaces, and nothing else", () => {
    const test = [
      'vi.mock("../../src/runtime/r.js");',
      'const actual = await vi.importActual<typeof import("../../src/runtime/r.js")>("../../src/runtime/r.js");',
      'const { r } = await import("../../src/runtime/r.js");',
    ].join("\n");
    expect(violations({ "test/api/a.test.ts": test, "src/runtime/r.ts": "" })).toEqual(["test/api/a.test.ts:3 [entry-points]"]);
  });
});
