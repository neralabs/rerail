import { describe, expect, it } from "vitest";
import { rerailRunsIn } from "../../../src/engine/catalog/repository-model.js";

describe("whether Rerail runs in a start state", () => {
  it("runs started or not set up, and not stopped or on an answer it cannot read", () => {
    expect(rerailRunsIn("started")).toBe(true);
    expect(rerailRunsIn("not-set-up")).toBe(true);
    expect(rerailRunsIn("stopped")).toBe(false);
    expect(rerailRunsIn("")).toBe(false);
  });
});
