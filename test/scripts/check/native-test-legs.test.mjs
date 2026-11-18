// scripts/check/native-test-legs.mjs: the Rust test legs, the check that each test is in exactly one, and what the
// legs take from the build (cargo's environment for a binary, libtest's lists and summaries).

import { describe, expect, it } from "vitest";
import { LEGS, coverProblems, envDelta, legArgs, parseList, parseResult } from "../../../scripts/check/native-test-legs.mjs";

/** A map of binary to tests, from an object of binary to an array of tests. */
const tests = (object) => new Map(Object.entries(object).map(([binary, names]) => [binary, new Set(names)]));

describe("the legs", () => {
  it("end with a leg that skips every path the others select", () => {
    const rest = LEGS.at(-1);
    expect(rest.filters).toEqual([]);
    expect(rest.skip).toEqual(LEGS.slice(0, -1).flatMap((leg) => leg.filters));
  });

  it("have distinct ids", () => {
    expect(new Set(LEGS.map((leg) => leg.id)).size).toBe(LEGS.length);
  });

  it("pass libtest the filters, then each skip", () => {
    expect(legArgs({ filters: ["a::", "b::"], skip: ["a::c::"] })).toEqual(["a::", "b::", "--skip", "a::c::"]);
  });
});

describe("coverProblems", () => {
  const all = tests({ engine: ["a::one", "b::two"], crate: ["three"] });

  it("passes a cover where each test is in one leg", () => {
    const byLeg = new Map([
      ["a", tests({ engine: ["a::one"], crate: [] })],
      ["rest", tests({ engine: ["b::two"], crate: ["three"] })],
    ]);
    expect(coverProblems(all, byLeg)).toEqual([]);
  });

  it("names a test in no leg, one in two, and one the whole list lacks", () => {
    const byLeg = new Map([
      ["a", tests({ engine: ["a::one", "b::two"], crate: [] })],
      ["rest", tests({ engine: ["b::two", "gone"], crate: [] })],
    ]);
    expect(coverProblems(all, byLeg)).toEqual([
      "engine: b::two is in legs a, rest",
      "crate: three is in no leg",
      "engine: leg rest selects gone, which the whole list lacks",
    ]);
  });
});

describe("envDelta", () => {
  it("keeps what cargo set or changed, never PATH or the script's own variables", () => {
    const parent = { HOME: "/home", PATH: "/bin", KEPT: "1" };
    const env = { HOME: "/home", PATH: "/cargo/bin:/bin", KEPT: "2", CARGO_MANIFEST_DIR: "/r/native", NATIVE_TESTS_CAPTURE: "/x" };
    expect(envDelta(env, parent)).toEqual({ KEPT: "2", CARGO_MANIFEST_DIR: "/r/native" });
  });
});

describe("libtest's output", () => {
  it("lists the tests, not the benchmarks", () => {
    expect(parseList("a::one: test\nb::two: test\nc: bench\n")).toEqual(["a::one", "b::two"]);
  });

  it("reads a summary line", () => {
    expect(parseResult("test result: FAILED. 3 passed; 1 failed; 2 ignored; 0 measured; 9 filtered out; finished in 1.00s")).toEqual({
      passed: 3, failed: 1, ignored: 2, measured: 0,
    });
    expect(parseResult("test a::one ... ok")).toBeUndefined();
  });
});
