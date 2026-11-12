import { describe, expect, it } from "vitest";
import { scriptBundleOptions } from "../support/build/script-bundle.js";
import { linesRewrittenForOlderNode } from "../support/platform/older-node.js";

/**
 * Once the Node set-up recorded is gone, the hooks fall back to `node` on PATH: Codex's and Cursor's through their shell
 * command, Claude Code's through `sh` or, on Windows, the hook launcher. On a Node older than Rerail needs, each hook
 * then says so on stderr and answers `{}` (`serveHook`), but only once its bundle has parsed: a bundle holding syntax
 * that Node lacks stops on a SyntaxError first, which an agent shows as a failed hook.
 */
describe("the hook bundles", () => {
  it.each([
    { script: "build:claude", entry: "src/hosts/claude/claude-hook.ts" },
    { script: "build:codex", entry: "src/hosts/codex/codex-hook.ts" },
    { script: "build:cursor", entry: "src/hosts/cursor/cursor-hook.ts" },
  ])("hold nothing an older Node cannot parse: $entry", async ({ script, entry }) => {
    expect(await linesRewrittenForOlderNode(await scriptBundleOptions(script, entry, "."))).toEqual([]);
  }, 60_000);
});
