import { describe, expect, it } from "vitest";
import { documentShape, portSize } from "./port-size.js";

const doc = (rules: unknown[]): unknown => ({
  format: "rerail.policy/v1",
  id: "tslint:example",
  checks: rules.map((rule, index) => ({ id: `c${index}`, forbid: { ast: { language: "typescript", rule } } })),
});

describe("documentShape", () => {
  it("counts a leaf rule as depth 1 with no relations", () => {
    expect(documentShape(doc([{ kind: "debugger_statement" }]))).toEqual({
      checks: 1,
      relations: 0,
      inside: 0,
      has: 0,
      precedes: 0,
      follows: 0,
      not: 0,
      any: 0,
      all: 0,
      maxDepth: 1,
    });
  });

  it("counts relations, not, any and all through nested rule objects", () => {
    const rule = {
      all: [
        { pattern: "super($$$A)" },
        { inside: { stopBy: "end", kind: "method_definition", has: { kind: "identifier" } } },
        { not: { follows: { any: [{ kind: "a" }, { precedes: { kind: "b" } }] } } },
      ],
    };
    // all -> not -> follows -> any -> precedes -> leaf = 6 levels.
    expect(documentShape(doc([rule]))).toMatchObject({ checks: 1, relations: 4, inside: 1, has: 1, precedes: 1, follows: 1, not: 1, any: 1, all: 1, maxDepth: 6 });
  });

  it("takes the deepest check and ignores relation-named keys outside rule trees", () => {
    const shape = documentShape({
      ...(doc([{ kind: "x" }, { has: { has: { kind: "y" } } }]) as object),
      description: "has inside not",
      scope: { include: ["**/*"], exclude: [] },
    });
    expect(shape).toMatchObject({ checks: 2, has: 2, relations: 2, maxDepth: 3 });
  });

  it("finds rules nested under select_when", () => {
    const shape = documentShape({ checks: [{ id: "c", forbid: { select_when: { select: { ast: { rule: { not: { kind: "z" } } } } } } }] });
    expect(shape).toMatchObject({ checks: 1, not: 1, maxDepth: 2 });
  });
  it("counts utility and constraint rules like the query's rule", () => {
    const shape = documentShape({
      checks: [{
        id: "c",
        forbid: { ast: { language: "typescript", rule: { matches: "u" }, utils: { u: { inside: { matches: "u" } } }, constraints: { A: { not: { has: { kind: "z" } } } } } },
      }],
    });
    expect(shape).toMatchObject({ checks: 1, relations: 2, inside: 1, has: 1, not: 1, maxDepth: 3 });
  });
});

describe("portSize", () => {
  it("sums counts and bytes over the default doc and overrides, keeps the deepest depth", () => {
    const base = JSON.stringify(doc([{ inside: { kind: "a" } }]));
    const override = JSON.stringify(doc([{ kind: "b" }, { not: { has: { has: { kind: "c" } } } }]));
    expect(portSize(base, [override])).toEqual({
      defaultBytes: base.length,
      totalBytes: base.length + override.length,
      docs: 2,
      checks: 3,
      relations: 3,
      inside: 1,
      has: 2,
      precedes: 0,
      follows: 0,
      not: 1,
      any: 0,
      all: 0,
      maxDepth: 4,
    });
  });

  it("counts bytes of an unparsable doc and reports no default for override-only ports", () => {
    expect(portSize(undefined, ["{not json"])).toMatchObject({ defaultBytes: 0, totalBytes: 9, docs: 1, checks: 0, maxDepth: 0 });
  });
});
