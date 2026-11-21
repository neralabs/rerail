import { mkdirSync, writeFileSync } from "node:fs";
import { mkdtemp } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";

import { nameDiff, readStatus, submoduleChanged, UNASKED, worktreeIndex } from "../../../src/engine/git-guard/submodule-state.js";
import { removeFolder } from "../../support/platform/remove-folder.js";

/** The audit's `git diff` (`audit-control.ts`) and the scan's (`headless-scan.ts`), as each runs it. */
const AUDIT_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--ignore-submodules=none", "--diff-filter=ACMRTUXB", "-z", "--"];
const SCAN_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--diff-filter=ACMRTUXB", "-z", "HEAD", "--"];

const roots: string[] = [];
afterEach(async () => {
  await Promise.all(roots.splice(0).map(root => removeFolder(root)));
});

describe("nameDiff and readStatus", () => {
  it.each([
    [AUDIT_DIFF, { relative: true, ignore: "none", listsModified: true }],
    [SCAN_DIFF, { relative: true, ignore: undefined, listsModified: true }],
    [["diff", "--name-only", "-z", "--diff-filter=d", "--"], { relative: false, ignore: undefined, listsModified: true }],
    [["diff", "--name-only", "-z", "--diff-filter=AD", "--"], { relative: false, ignore: undefined, listsModified: false }],
    [["diff", "--name-only", "-z", "--merge-base", "main", "--"], { relative: false, ignore: undefined, listsModified: true }],
    [["diff", "--name-only", "-z", "--", "a.txt"], undefined],
    [["diff", "--name-only", "-z"], undefined],
    [["diff", "--name-status", "-z", "--"], undefined],
    [["diff", "--name-only", "-z", "--relative=src", "--"], undefined],
    [["diff", "--name-only", "-z", "HEAD~1", "HEAD", "--"], undefined],
    [["diff", "--name-only", "-z", "HEAD~1..HEAD", "--"], undefined],
    [["diff", "--name-only", "-z", "HEAD^!", "--"], undefined],
    [["diff", "--name-only", "-z", "--merge-base", "main", "HEAD", "--"], undefined],
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

describe("submoduleChanged", () => {
  it("fails, opening nothing, for a checked-out submodule past the depth Rerail follows", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-submodule-depth-"));
    roots.push(root);
    // A Git folder as Git's `is_git_directory` takes one: a `HEAD`, `objects` and `refs`.
    for (const name of ["objects", "refs"]) mkdirSync(join(root, "sub", ".git", name), { recursive: true });
    writeFileSync(join(root, "sub", ".git", "HEAD"), "ref: refs/heads/main\n");
    const opened: string[] = [];

    const changed = await submoduleChanged(join(root, "sub"), false, async folder => {
      opened.push(folder);
      return await Promise.resolve(UNASKED);
    }, 0);

    expect([typeof changed === "object" && changed.error?.message, opened]).toEqual([`Rerail follows submodules 16 levels deep, and ${join(root, "sub")} is deeper`, []]);
  });
});

describe("worktreeIndex", () => {
  it("leaves the index of a folder whose real path cannot be read to Git", async () => {
    const root = await mkdtemp(join(tmpdir(), "rerail-worktree-index-"));
    roots.push(root);

    expect(worktreeIndex(join(root, "gone"), {})).toBeUndefined();
  });
});
