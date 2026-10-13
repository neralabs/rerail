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

  it("flags forbidden and control characters, reserved device names, and trailing dots or spaces", () => {
    const problems = windowsPathProblems([
      "fixtures/a/what?.js",
      "fixtures/a/b:c/d.js",
      "fixtures/a/tab\there.js",
      "fixtures/a/AUX.js",
      "fixtures/con/x.js",
      "fixtures/a/nul .js",
      "fixtures/a/COM0.js",
      "fixtures/a/com¹.js",
      "fixtures/a/LPT0",
      "fixtures/a/lpt².txt",
      "fixtures/a/CONIN$.js",
      "fixtures/a/conout$",
      "fixtures/a/trailing./x.js",
      "fixtures/a/space /x.js",
      "fixtures/a/console.js",
      "fixtures/a/com10.js",
      "fixtures/a/[id]/$user.js",
    ]);
    expect(paths(problems).sort()).toEqual(
      [
        "fixtures/a/AUX.js",
        "fixtures/a/CONIN$.js",
        "fixtures/a/COM0.js",
        "fixtures/a/LPT0",
        "fixtures/a/b:c/d.js",
        "fixtures/a/com¹.js",
        "fixtures/a/conout$",
        "fixtures/a/lpt².txt",
        "fixtures/a/nul .js",
        "fixtures/a/space /x.js",
        "fixtures/a/tab\there.js",
        "fixtures/a/trailing./x.js",
        "fixtures/a/what?.js",
        "fixtures/con/x.js",
      ].sort(),
    );
  });

  it("flags files and directories whose paths differ only by case, once per clash", () => {
    const problems = windowsPathProblems([
      "fixtures/Src/a.js",
      "fixtures/src/b.js",
      "fixtures/src/c.js",
      "x/Readme.md",
      "x/README.md",
      "y/Foo",
      "y/foo/x.js",
      "z/a.js",
    ]);
    expect(problems.map((problem) => problem.message)).toEqual([
      "fixtures/src and fixtures/Src differ only by case; Windows sees one path",
      "x/Readme.md and x/README.md differ only by case; Windows sees one path",
      "y/foo and y/Foo differ only by case; Windows sees one path",
    ]);
  });
});
