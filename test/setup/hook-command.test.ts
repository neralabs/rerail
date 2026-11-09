import { join, win32 } from "node:path";
import { describe, expect, it } from "vitest";
import { generatedHookTarget, windowsCmdHookCommand } from "../../src/setup/harness-registration.js";

// Pure functions only: nothing here registers a hook or writes a coding agent's settings.
const CURSOR_HOOK = join("dist", "hosts", "cursor", "cursor-hook.js");

describe("the Windows cmd hook command", () => {
  const node = String.raw`C:\Program Files\nodejs\node.exe`;
  const root = String.raw`C:\Users\me\.rerail\current`;
  const hook = win32.join(root, CURSOR_HOOK);

  it("falls back to node.exe from PATH alone, never a bare node cmd looks for in the current folder", () => {
    const command = windowsCmdHookCommand(`"${node}"`, `"${hook}"`);
    expect(command).toBe(`cmd /d /c "if exist "${node}" ("${node}" "${hook}") else (for %I in (node.exe) do @"%~$PATH:I" "${hook}")"`);
    expect(command).not.toContain("(node ");
  });

  it.runIf(process.platform === "win32")("recognizes the form it writes and the forms earlier releases wrote", () => {
    const written = windowsCmdHookCommand(`"${node}"`, `"${hook}"`);
    expect(generatedHookTarget(written, CURSOR_HOOK, "win32")).toEqual({ nodePath: node, script: hook });
    const legacy = `cmd /d /c "if exist "${node}" ("${node}" "${hook}") else (node "${hook}")"`;
    expect(generatedHookTarget(legacy, CURSOR_HOOK, "win32")).toEqual({ nodePath: node, script: hook });
    // Releases before this one doubled each `%` in a path, which only a batch file turns back into one.
    const percentNode = String.raw`C:\Users\a%b\node.exe`;
    const doubled = `cmd /d /c "if exist "${percentNode.replaceAll("%", "%%")}" ("${percentNode.replaceAll("%", "%%")}" "${hook}") else (node "${hook}")"`;
    expect(generatedHookTarget(doubled, CURSOR_HOOK, "win32")).toEqual({ nodePath: percentNode, script: hook });
    expect(generatedHookTarget(`cmd /d /c "if exist "${node}" ("${node}" "${hook}") else (calc)"`, CURSOR_HOOK, "win32")).toBeUndefined();
  });
});
