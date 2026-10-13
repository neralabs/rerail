// scripts/check/check-windows-paths.mjs: every tracked path checks out on Windows.

import { describe, expect, it } from "vitest";
import { MAX_LENGTH, windowsPathProblems } from "../../scripts/check/check-windows-paths.mjs";

const paths = (problems) => problems.map((problem) => problem.path);
/** A path of exactly `length` characters. */
const ofLength = (length, prefix = "packs/a/upstream/") => `${prefix}${"x".repeat(length - prefix.length - ".js".length)}.js`;

describe("check-windows-paths", () => {
  it("allows a path of the budget's length and flags one character more", () => {
    const longest = ofLength(MAX_LENGTH);
    const over = ofLength(MAX_LENGTH + 1, "packs/b/upstream/");
    const problems = windowsPathProblems([longest, over]);
    expect(problems).toEqual([
      { path: over, message: `${MAX_LENGTH + 1} characters; Windows checkouts allow ${MAX_LENGTH} (its 260 less the clone directory)` },
    ]);
  });

  it("flags forbidden characters, reserved device names in any case or before an extension, and trailing dots or spaces", () => {
    const problems = windowsPathProblems([
      "fixtures/a/what?.js",
      "fixtures/a/b:c/d.js",
      "fixtures/a/AUX.js",
      "fixtures/con/x.js",
      "fixtures/a/trailing./x.js",
      "fixtures/a/space /x.js",
      "fixtures/a/console.js",
      "fixtures/a/[id]/$user.js",
    ]);
    expect(paths(problems).sort()).toEqual([
      "fixtures/a/AUX.js",
      "fixtures/a/b:c/d.js",
      "fixtures/a/space /x.js",
      "fixtures/a/trailing./x.js",
      "fixtures/a/what?.js",
      "fixtures/con/x.js",
    ]);
  });

  it("flags files and directories whose paths differ only by case", () => {
    const problems = windowsPathProblems(["fixtures/Src/a.js", "fixtures/src/b.js", "x/Readme.md", "x/README.md", "y/a.js"]);
    expect(problems.map((problem) => problem.message)).toEqual([
      "fixtures/src and fixtures/Src differ only by case; Windows sees one path",
      "x/Readme.md and x/README.md differ only by case; Windows sees one path",
    ]);
  });
});
