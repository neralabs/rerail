import { describe, expect, it } from "vitest";
import { noticeCounts } from "../../../../src/engine/code-rule/worktree-notices.js";

/** A read that walked the worktree, `src` and `src/deep`, and left out an ignored folder, an ignored file and a worktree. */
const seen = { folders: new Set(["", "src", "src/deep"]), leftOut: new Set(["node_modules", "src/cache.tmp", "nested"]) };

describe("noticeCounts", () => {
  it.each([
    ["change", "src/a.py", true, "a file the read observed changed"],
    ["rename", "src/new.py", true, "a file came or went in a folder the read walked"],
    ["rename", "src/fresh", true, "a folder came"],
    ["change", "src/fresh/n.py", true, "a file changed in a folder made since the read"],
    ["change", "PKG_0~1/SUB_3~1/M0.PY", true, "a short name the read never saw"],
    ["rename", ".git", true, "a checkout came or went"],
    ["rename", "src/deep/.git", true, "a checkout came or went inside"],
    ["rename", "node_modules", true, "what the read left out came or went"],
    ["change", "src", false, "the times of a folder the read walked, whose entries' changes come on their own"],
    ["change", ".git", false, "the times of a .git folder"],
    ["change", ".git/index", false, "Git's own files"],
    ["rename", ".git/index.lock", false, "Git's own files"],
    ["change", "node_modules", false, "the times of an ignored folder"],
    ["rename", "node_modules/pkg/index.js", false, "a file in an ignored folder"],
    ["change", "src/cache.tmp", false, "an ignored file"],
    ["change", "nested/src/a.py", false, "a file in another worktree inside"],
  ])("%s of %s counts: %s (%s)", (event, path, counts) => {
    expect(noticeCounts(seen, event, path)).toBe(counts);
  });
});
