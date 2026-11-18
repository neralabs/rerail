// scripts/ship/engine-json.ts: the engine's stdout is its JSON document followed by its timing lines (decision 0288);
// the harness reads the document and nothing after it, braces inside strings included.

import { describe, expect, it } from "vitest";
import { engineJson } from "../../../scripts/ship/engine-json.js";

describe("the engine's JSON output", () => {
  it("reads the document and leaves the engine's timing lines after it", () => {
    const stdout = '{"nodes":82583,"note":"a } and a ] in a string, and an escaped \\" quote"}\n  after the run 1.20 s wall: the command\'s thread 1.10 s own\n  before the command: 0.02 s since the process started\n';
    expect(engineJson(stdout)).toEqual({ nodes: 82583, note: 'a } and a ] in a string, and an escaped " quote' });
  });

  it("reads an array document, as check-rr writes", () => {
    expect(engineJson('[{"path":"a.rerail","status":"reader-passed"}]\n  after the run 0.10 s wall\n')).toEqual([{ path: "a.rerail", status: "reader-passed" }]);
  });

  it("fails on output that holds no document", () => {
    expect(() => engineJson("  after the run 0.10 s wall\n")).toThrow();
  });
});
