import { describe, expect, it } from "vitest";
import { escapeInvisible, lineDiff, visibleJson, type DiffLine } from "../../ui/rule-diff.js";

/** The text each side of a diff reads back to: every line the side has, in order. */
const side = (lines: readonly DiffLine[], drop: "added" | "removed") =>
  lines.filter(line => line.op !== drop).map(line => line.text).join("\n");

describe("a rule's content as a person reads it", () => {
  it("shows every invisible character as an escape, and leaves visible text alone", () => {
    const hidden = "a\u200bb\u202ec\u{e0041}d\ue000e";
    expect(escapeInvisible(hidden)).toBe("a\\u200bb\\u202ec\\u{e0041}d\\ue000e");
    expect(escapeInvisible("Use the logger — not print, ünïcode ✓")).toBe("Use the logger — not print, ünïcode ✓");
  });

  it("escapes invisible characters in keys and nested values of the JSON", () => {
    const text = visibleJson({ clauses: [{ message: "log\u2066 here" }], "i\u200bd": "x" });
    expect(text).toContain("log\\u2066 here");
    expect(text).toContain("i\\u200bd");
    expect(text).not.toMatch(/[\p{Cf}\p{Co}\p{Cn}]/u);
  });

  it("reads the same whatever order the keys came in, so only real changes differ", () => {
    const one = { id: "r", scope: { include: ["**/*.ts"], exclude: [] }, mode: "log" };
    const other = { mode: "log", scope: { exclude: [], include: ["**/*.ts"] }, id: "r" };
    expect(visibleJson(one)).toBe(visibleJson(other));
    expect(visibleJson(one).split("\n").length).toBeGreaterThan(3);
  });
});

describe("a line diff", () => {
  it("marks nothing when the content is unchanged", () => {
    const text = visibleJson({ id: "r", message: "same" });
    expect(lineDiff(text, text).every(line => line.op === "same")).toBe(true);
  });

  it("keeps every line of both sides, in order, and marks only the changed ones", () => {
    const before = visibleJson({ id: "r", message: "old words", fix: "a", scope: ["src"] });
    const after = visibleJson({ id: "r", message: "new words", fix: "a", scope: ["src", "lib"] });
    const lines = lineDiff(before, after);

    expect(side(lines, "removed")).toBe(after);
    expect(side(lines, "added")).toBe(before);
    expect(lines.filter(line => line.op === "removed").map(line => line.text.trim())).toEqual(["\"message\": \"old words\",", "\"src\""]);
    expect(lines.filter(line => line.op === "added").map(line => line.text.trim())).toEqual(["\"message\": \"new words\",", "\"src\",", "\"lib\""]);
  });

  it("puts a replaced line's old text right before its new text", () => {
    const lines = lineDiff("a\nb\nc", "a\nB\nc");
    expect(lines).toEqual([
      { op: "same", text: "a" }, { op: "removed", text: "b" }, { op: "added", text: "B" }, { op: "same", text: "c" },
    ]);
  });

  it("handles content added or removed at either end", () => {
    expect(lineDiff("b", "a\nb\nc")).toEqual([{ op: "added", text: "a" }, { op: "same", text: "b" }, { op: "added", text: "c" }]);
    expect(lineDiff("a\nb\nc", "b")).toEqual([{ op: "removed", text: "a" }, { op: "same", text: "b" }, { op: "removed", text: "c" }]);
  });
});
