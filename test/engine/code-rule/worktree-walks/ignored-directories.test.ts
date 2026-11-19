import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { IGNORED_DIRECTORIES_RUST_PATH, renderIgnoredDirectoriesRust } from "../../../../scripts/generate/walk/ignored-directories.js";

describe("the ignored directories", () => {
  it("are the engine's: its generated list is the host's (npm run generate:ignored-directories)", () => {
    expect(readFileSync(IGNORED_DIRECTORIES_RUST_PATH, "utf8")).toBe(renderIgnoredDirectoriesRust());
  });
});
