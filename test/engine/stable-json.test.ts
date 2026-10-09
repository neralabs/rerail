import { describe, expect, it } from "vitest";
import { stableJsonText } from "../../src/engine/stable-json.js";

describe("stableJsonText", () => {
  it("sorts object keys recursively without reordering arrays", () => {
    expect(stableJsonText({ z: 2, nested: { y: false, x: true }, a: [3, { d: 4, c: 5 }] })).toBe(
      '{"a":[3,{"c":5,"d":4}],"nested":{"x":true,"y":false},"z":2}',
    );
  });

  it("orders keys by Unicode code point", () => {
    expect(stableJsonText({ "\uE000": "private-use", "😀": "emoji" })).toBe(
      '{"\uE000":"private-use","😀":"emoji"}',
    );
  });

  it("duplicates repeated non-cyclic references", () => {
    const shared = { z: 2, a: 1 };
    expect(stableJsonText({ right: shared, left: shared })).toBe('{"left":{"a":1,"z":2},"right":{"a":1,"z":2}}');
  });

  it("normalizes values through JSON serialization semantics", () => {
    expect(stableJsonText({ z: new Date("2024-01-02T03:04:05.000Z"), a: 1 })).toBe(
      '{"a":1,"z":"2024-01-02T03:04:05.000Z"}',
    );
  });

  it("returns undefined for values JSON cannot serialize", () => {
    const cyclic: { self?: unknown } = {};
    cyclic.self = cyclic;
    expect(stableJsonText(undefined)).toBeUndefined();
    expect(stableJsonText(cyclic)).toBeUndefined();
    expect(stableJsonText(1n)).toBeUndefined();
  });
});
