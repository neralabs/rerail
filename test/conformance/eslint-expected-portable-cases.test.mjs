/**
 * What the ESLint-based generators share to keep the generating machine out of fixtures
 * (scripts/pack-data/eslint-expected.mjs): the working directory upstream tests see while they declare their cases,
 * and the guard every case-writing generator calls before it writes a fixture; and the check that an authored case's
 * expected findings name only rules the expected findings cover.
 */
import { describe, expect, it } from "vitest";
import { assertKnownReportRules, assertPortableCase, UPSTREAM_TEST_CWD, withUpstreamTestCwd } from "../../scripts/pack-data/eslint-expected.mjs";

/** The scratch directory #31's unicorn generator ran in, in `machineDirectories()`' form. */
const SCRATCH = "private/tmp/claude-501/-Users-gabrielgiangi-Documents-Repositories-nera-rule-discovery/0b8f1b38-a217-4238-ab1f-806b5bc153c8/scratchpad/work/javascript";

describe("withUpstreamTestCwd", () => {
  it("answers UPSTREAM_TEST_CWD while upstream tests declare their cases, and the real directory again afterwards, even when they throw", async () => {
    const real = process.cwd();
    expect(await withUpstreamTestCwd(async () => process.cwd())).toBe(UPSTREAM_TEST_CWD);
    expect(process.cwd()).toBe(real);
    await expect(withUpstreamTestCwd(() => Promise.reject(new Error("upstream test failed")))).rejects.toThrow("upstream test failed");
    expect(process.cwd()).toBe(real);
  });
});

describe("assertPortableCase", () => {
  it("refuses a case whose fixture path or code carries the generating machine's directory, as #31's filename-case did", () => {
    const parent = SCRATCH.slice(0, SCRATCH.lastIndexOf("/"));
    expect(() => assertPortableCase(`filename-case/112/${parent}/Src/fooBar.js`, "", [SCRATCH, parent])).toThrow(/fixture path .* carries this machine's directory/u);
    expect(() => assertPortableCase("filename-case/143/work/project/src/FooBar/file.js", `/* Filename: /${SCRATCH}/src/FooBar/file.js */`, [SCRATCH])).toThrow(
      /code carries this machine's directory/u,
    );
  });

  it("accepts what the pinned working directory records, and paths that only share a prefix with the machine's", () => {
    expect(() => assertPortableCase("filename-case/143/work/project/src/FooBar/file.js", "/* Filename: /work/project/src/FooBar/file.js */", [SCRATCH])).not.toThrow();
    expect(() => assertPortableCase("rule/valid-001/Users/jeffrey/app.js", "require('/Users/jeffrey/app')", ["Users/jeff"])).not.toThrow();
  });

  it("finds a Windows home directory in code written with a drive letter and backslashes", () => {
    expect(() => assertPortableCase("rule/valid-001.js", String.raw`const p = "C:\Users\bob\project\a.js";`, ["Users/bob"])).toThrow(/code carries/u);
  });
});

describe("assertKnownReportRules", () => {
  const rules = ["expect-expect", "no-disabled-tests", "no-focused-tests"];

  it("refuses expected findings for a rule the expected findings do not cover, such as a misspelled one that would read as 0", () => {
    expect(() => assertKnownReportRules("unchecked-heads", { "no-disabled-test": 2 }, rules)).toThrow(
      "AUTHORED unchecked-heads: reports names no-disabled-test, which is not a pack rule",
    );
  });

  it("accepts expected findings for the expected findings' rules, and none at all", () => {
    expect(() => assertKnownReportRules("vitest-import", { "expect-expect": 2, "no-focused-tests": 1 }, rules)).not.toThrow();
    expect(() => assertKnownReportRules("node-test", {}, rules)).not.toThrow();
  });
});
