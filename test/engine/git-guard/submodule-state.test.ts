import { describe, expect, it } from "vitest";

import { nameDiff, readStatus } from "../../../src/engine/git-guard/submodule-state.js";

/** The audit's `git diff` (`audit-control.ts`) and the scan's (`headless-scan.ts`), as each runs it. */
const AUDIT_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--ignore-submodules=none", "--diff-filter=ACMRTUXB", "-z", "--"];
const SCAN_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--diff-filter=ACMRTUXB", "-z", "HEAD", "--"];

describe("nameDiff and readStatus", () => {
  it.each([
    [AUDIT_DIFF, { relative: true, ignore: "none", listsModified: true }],
    [SCAN_DIFF, { relative: true, ignore: undefined, listsModified: true }],
    [["diff", "--name-only", "-z", "--diff-filter=d", "--"], { relative: false, ignore: undefined, listsModified: true }],
    [["diff", "--name-only", "-z", "--diff-filter=AD", "--"], { relative: false, ignore: undefined, listsModified: false }],
    [["diff", "--name-only", "-z", "--", "a.txt"], undefined],
    [["diff", "--name-status", "-z", "--"], undefined],
    [["diff", "--name-only", "-z", "--relative=src", "--"], undefined],
  ] as const)("%j is %j", (args, expected) => {
    expect(nameDiff(args)).toEqual(expected);
  });

  it("tells a submodule's own entries from those of the submodules inside it", () => {
    const sha = "0".repeat(40);
    const output = [
      `1 .M SC.. 160000 160000 160000 ${sha} ${sha} inner one`,
      `1 M. S... 160000 160000 160000 ${sha} ${sha} staged`,
      `2 R. N... 100644 100644 100644 ${sha} ${sha} R100 new name`, "old name",
    ].join("\0");

    const read = readStatus(`${output}\0`);

    expect(read.own).toBe(true);
    expect([...read.submodules]).toEqual([["inner one", { staged: false, moved: true }], ["staged", { staged: true, moved: false }]]);
    expect(readStatus(`1 .M SC.. 160000 160000 160000 ${sha} ${sha} only\0`).own).toBe(false);
  });
});
