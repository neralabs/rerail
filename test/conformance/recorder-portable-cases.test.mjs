/**
 * What the ESLint-based recorders share to keep the recording machine out of fixtures
 * (conformance/recorders/eslint-record.mjs): the working directory upstream tests see while they declare their cases,
 * and the guard every case-writing recorder calls before it writes a fixture.
 */
import { describe, expect, it } from "vitest";
import { assertPortableCase, RECORDING_CWD, withRecordingCwd } from "../../conformance/recorders/eslint-record.mjs";

/** The scratch directory #31's unicorn recording ran in, in `machineDirectories()`' form. */
const SCRATCH = "private/tmp/claude-501/-Users-gabrielgiangi-Documents-Repositories-nera-rule-discovery/0b8f1b38-a217-4238-ab1f-806b5bc153c8/scratchpad/work/javascript";

describe("withRecordingCwd", () => {
  it("answers RECORDING_CWD while upstream tests declare their cases, and the real directory again afterwards, even when they throw", async () => {
    const real = process.cwd();
    expect(await withRecordingCwd(async () => process.cwd())).toBe(RECORDING_CWD);
    expect(process.cwd()).toBe(real);
    await expect(withRecordingCwd(() => Promise.reject(new Error("upstream test failed")))).rejects.toThrow("upstream test failed");
    expect(process.cwd()).toBe(real);
  });
});

describe("assertPortableCase", () => {
  it("refuses a case whose fixture path or code carries the recording machine's directory, as #31's filename-case did", () => {
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
