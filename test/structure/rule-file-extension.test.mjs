// Rule files are named `.rerail`; `.rr` was the extension until 2026-11-17. The engine and the CLI read `.rerail`
// only, so a `.rr` file a merge or an older branch brings back is a rule no run reads.

import { execFileSync } from "node:child_process";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import { describe, expect, it } from "vitest";

const root = resolve(dirname(fileURLToPath(import.meta.url)), "..", "..");

describe("rule files", () => {
  it("are named .rerail: no tracked file keeps the retired .rr extension", () => {
    const retired = execFileSync("git", ["ls-files", "-z", "--", "*.rr", "*.rr.sample"], { cwd: root, maxBuffer: 64 * 1024 * 1024 })
      .toString("utf8")
      .split("\0")
      .filter(Boolean);
    expect(retired, "rename each with `git mv <file>.rr <file>.rerail` and its `#include` lines").toEqual([]);
  });
});
