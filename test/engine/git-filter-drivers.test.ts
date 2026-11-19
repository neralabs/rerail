import { describe, expect, it } from "vitest";

import { driverOverrides, listedEntries, readDriverConfig, worktreeRead } from "../../src/engine/git-filter-drivers.js";

/** The audit's `git diff` (`audit-control.ts`), the scan's (`headless-scan.ts`) and the session walk's listing, as each runs it. */
const AUDIT_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--ignore-submodules=none", "--diff-filter=ACMRTUXB", "-z", "--"];
const SCAN_DIFF = ["diff", "--name-only", "--no-renames", "--relative", "--diff-filter=ACMRTUXB", "-z", "HEAD", "--"];
const WALK_LISTING = ["ls-files", "-z", "--modified", "--others", "--exclude-standard"];

describe("worktreeRead", () => {
  it.each([
    [AUDIT_DIFF, "files-and-submodules"],
    [SCAN_DIFF, "files-and-submodules"],
    [["-c", "core.quotePath=false", "diff", "--name-only", "-z", "--"], "files-and-submodules"],
    [["diff", "--cached", "--name-only", "--ignore-submodules=none", "-z", "--"], "none"],
    [["diff", "--name-only", "--ignore-submodules=dirty", "--", "--cached"], "files"],
    [WALK_LISTING, "files"],
    [["ls-files", "-z", "-t", "--cached", "--others", "--exclude-standard"], "none"],
    [["--literal-pathspecs", "ls-files", "--stage", "-z", "--", "a"], "none"],
    [["cat-file", "-e", "HEAD:a"], "none"],
    [["cat-file", "--batch", "--filters"], "files"],
    [["worktree", "list", "--porcelain"], "none"],
    [["worktree", "add", "x"], "files"],
    [["status", "--porcelain=2", "--ignore-submodules=dirty"], "files"],
    [["status", "--porcelain"], "files-and-submodules"],
    [["hash-object", "-w", "a.txt"], "files"],
    [["reflog", "show", "--format=%H %gs", "HEAD"], "none"],
  ] as const)("%j reads %s", (args, read) => {
    expect(worktreeRead(args)).toBe(read);
  });
});

describe("driverOverrides", () => {
  const listing = (entries: ReadonlyArray<readonly [string, string, string | undefined]>): string =>
    entries.map(([scope, key, value]) => `${scope}\0${key}${value === undefined ? "" : `\n${value}`}\0`).join("");

  it("gives each driver a repository defines the person's values, or none, and leaves the person's other drivers alone", () => {
    const config = readDriverConfig(listedEntries(listing([
      ["system", "filter.lfs.process", "git-lfs filter-process"],
      ["global", "filter.lfs.required", "true"],
      ["global", "filter.mine.clean", "mine"],
      ["local", "filter.lfs.process", "repository's"],
      ["local", "filter.a.b.clean", "repository's"],
      ["worktree", "filter.lfs.smudge", "repository's"],
    ])));
    expect(driverOverrides(config)).toEqual([
      ["filter.lfs.smudge", ""], ["filter.lfs.process", "git-lfs filter-process"], ["filter.lfs.required", "true"],
      ["filter.a.b.clean", ""], ["filter.a.b.required", "false"],
    ]);
  });

  it("turns the person's driver off, not required, where the repository adds a process it lacks", () => {
    const config = readDriverConfig(listedEntries(listing([
      ["global", "filter.own.clean", "own"], ["global", "filter.own.required", "true"], ["local", "filter.own.process", "repository's"],
    ])));
    expect(driverOverrides(config)).toEqual([["filter.own.clean", "own"], ["filter.own.process", ""], ["filter.own.required", "false"]]);
  });

  it("keeps a `required` the person writes with no value, which Git reads as true", () => {
    const config = readDriverConfig(listedEntries(listing([
      ["global", "filter.own.clean", "own"], ["global", "filter.own.required", undefined], ["local", "filter.own.clean", "repository's"],
    ])));
    expect(driverOverrides(config)).toEqual([["filter.own.clean", "own"], ["filter.own.required", "true"]]);
  });
});
