import { describe, expect, it } from "vitest";
import { parseUnitLog, unitJob, unitTestTimes } from "../../scripts/generate/tests/unit-test-times.mjs";

/** A unit test job's log as the Actions API returns it: a timestamp before each line, vitest's colours kept. */
function actionsLog(lines) {
  return lines.map((line, index) => `2026-11-16T20:44:${String(10 + index).padStart(2, "0")}.4996469Z ${line}`).join("\n");
}

const SHARD_ONE = actionsLog([
  "##[group]Run npm ci",
  // A file line outside the suite step counts for nothing.
  " \u001b[32m✓\u001b[39m test/elsewhere.test.ts \u001b[2m(1 test)\u001b[22m 999ms",
  "##[group]Run npm test -- --shard=1/3",
  " \u001b[32m✓\u001b[39m test/cli/a.test.ts \u001b[2m(61 tests)\u001b[22m\u001b[33m 60000ms\u001b[39m",
  "     \u001b[33m\u001b[2m✓\u001b[22m\u001b[39m waits for a busy daemon \u001b[33m 18400ms\u001b[39m",
  " \u001b[32m✓\u001b[39m test/b.test.mjs \u001b[2m(4 tests | 1 skipped)\u001b[22m 20000ms",
  " \u001b[33m↓\u001b[39m test/skipped.test.ts \u001b[2m(3 tests | 3 skipped)\u001b[22m",
  "\u001b[2m Test Files \u001b[22m 3 passed (3)",
  "\u001b[2m   Duration \u001b[22m 40.00s\u001b[2m (tests 91%, import 7%, transform 2%)\u001b[22m",
  "##[group]Run npm run test:coverage:lint-config",
  " \u001b[32m✓\u001b[39m test/cli/a.test.ts \u001b[2m(61 tests)\u001b[22m 1ms",
  "\u001b[2m   Duration \u001b[22m 10.00s\u001b[2m (tests 98%)\u001b[22m",
  "Post job cleanup.",
]);

// Shard 1 on Windows as CI logs it now: the suite step's header shows only its command's first line, and the line that
// passes vitest `--shard=` comes later in the command, echoed under the header.
const WINDOWS_SHARD_ONE = actionsLog([
  '##[group]Run read -ra tests <<< "$TESTS"',
  '\u001b[36;1mread -ra tests <<< "$TESTS"\u001b[0m',
  '\u001b[36;1mnode_modules/.bin/vitest run --dir test --shard=1/5 "$@"\u001b[0m',
  "env:",
  "  TESTS: ",
  "##[endgroup]",
  " \u001b[32m✓\u001b[39m test/engine/a.test.ts \u001b[2m(45 tests)\u001b[22m\u001b[33m 353000\u001b[2mms\u001b[22m\u001b[39m",
  "\u001b[2m   Duration \u001b[22m 300.00s\u001b[2m (tests 91%)\u001b[22m",
  "##[group]Run npx vitest run integration/claude-hook-rules.test.ts",
  " \u001b[32m✓\u001b[39m integration/claude-hook-rules.test.ts \u001b[2m(3 tests)\u001b[22m 50000ms",
  "\u001b[2m   Duration \u001b[22m 60.00s\u001b[2m (tests 98%)\u001b[22m",
]);

describe("unit-test-times", () => {
  it("knows the unit test jobs by name, and each one's platform and shard", () => {
    expect(unitJob("Unit tests (2/3)")).toEqual({ platform: "linux", shard: 2 });
    expect(unitJob("Unit tests (Windows, 1/5)")).toEqual({ platform: "win32", shard: 1 });
    expect(unitJob("Unit tests")).toBeUndefined();
    expect(unitJob("Rust engine (tests, the rest)")).toBeUndefined();
  });

  it("reads each file's time from the suite step alone, and the steps after it as extra time", () => {
    const log = parseUnitLog(SHARD_ONE);
    expect(Object.fromEntries(log.files)).toEqual({ "test/cli/a.test.ts": 60000, "test/b.test.mjs": 20000, "test/skipped.test.ts": 0 });
    expect(log.suiteMs).toBe(40000);
    expect(log.extraMs).toBe(10000);
  });

  it("knows the suite step by the --shard= of its command, on any line of a command of several lines", () => {
    const log = parseUnitLog(WINDOWS_SHARD_ONE);
    expect(Object.fromEntries(log.files)).toEqual({ "test/engine/a.test.ts": 353000 });
    expect(log.suiteMs).toBe(300000);
    expect(log.extraMs).toBe(60000);
  });

  it("refuses a platform whose jobs give no test file's time, rather than writing it with none", () => {
    const jobs = [
      { platform: "linux", shard: 1, log: { files: new Map([["a.test.ts", 5]]), suiteMs: 5, extraMs: 0 } },
      { platform: "win32", shard: 1, log: { files: new Map(), suiteMs: 0, extraMs: 0 } },
    ];
    expect(() => unitTestTimes(jobs, () => true)).toThrow(/win32/u);
  });

  it("keeps each file's median per platform, the files the checkout has, and the extra step as test-file time", () => {
    const log = (files, suiteMs = 0, extraMs = 0) => ({ files: new Map(Object.entries(files)), suiteMs, extraMs });
    const jobs = [
      { platform: "linux", shard: 1, log: log({ "a.test.ts": 30, "gone.test.ts": 5 }, 20, 10) },
      { platform: "linux", shard: 2, log: log({ "a.test.ts": 10, "b.test.ts": 7 }) },
      { platform: "linux", shard: 1, log: log({ "a.test.ts": 20 }, 10, 4) },
      { platform: "win32", shard: 1, log: log({ "a.test.ts": 90 }) },
    ];
    // Shard 1's suite ran 35 ms of files in 20 ms, then 20 ms in 10 ms: its extra steps count 10 × 1.75 and 4 × 2.
    expect(unitTestTimes(jobs, (path) => path !== "gone.test.ts")).toEqual({
      linux: { firstShardExtraMs: 13, files: { "a.test.ts": 20, "b.test.ts": 7 } },
      win32: { firstShardExtraMs: 0, files: { "a.test.ts": 90 } },
    });
  });
});
