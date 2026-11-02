/** The conformance command line (conformance/languages/cli.ts): the combinations it refuses before scoring anything. */
import { describe, expect, it } from "vitest";

import { parseCli } from "../../conformance/languages/cli.js";

describe("parseCli", () => {
  it("refuses --check with --pack-dir: the gates judge the repository's packs/, not the directory given", () => {
    expect(() => parseCli(["--check", "--pack-dir", "/tmp/some-pack"])).toThrow(/--check .* does not take --pack-dir/u);
  });

  it("scores a pack directory outside --check", () => {
    expect(parseCli(["--pack-dir", "/tmp/some-pack"]).directories).toEqual(["/tmp/some-pack"]);
  });
});
