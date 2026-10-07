import { describe, expect, it } from "vitest";
import { applyFixes, checkFix } from "./fix-check.js";

describe("applyFixes", () => {
  it("applies byte-range replacements from the end, so earlier offsets stay valid", () => {
    const source = "var a = 1;\nvar é = 2;\n";
    const fixes = [
      { startByte: 0, endByte: 3, text: "let" },
      { startByte: 11, endByte: 14, text: "let" },
    ];
    expect(applyFixes(source, fixes)).toBe("let a = 1;\nlet é = 2;\n");
  });

  it("counts offsets in UTF-8 bytes", () => {
    expect(applyFixes("é = undefined;", [{ startByte: 0, endByte: 14, text: "é" }])).toBe("é;");
  });
});

describe("checkFix", () => {
  it("reports the first differing line", () => {
    const result = checkFix("a\nb\n", [{ startByte: 2, endByte: 3, text: "c" }], "a\nd\n");
    expect(result).toEqual({ emitted: 1, matched: false, firstDiff: { line: 1, expected: "d", actual: "c" } });
  });

  it("matches when the fixed text is identical", () => {
    expect(checkFix("x", [], "x")).toEqual({ emitted: 0, matched: true });
  });
});
