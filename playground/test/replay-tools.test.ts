import { mkdir, mkdtemp, realpath, rm, symlink, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { afterEach, describe, expect, it } from "vitest";
import { replayTools, safePath, sandboxFilePath } from "../src/replay-tools.js";
import type { ExtensionAPI, ExtensionContext, ToolDefinition } from "@oh-my-pi/pi-coding-agent";

const directories: string[] = [];
afterEach(async () => {
  await Promise.all(directories.splice(0).map((path) => rm(path, { recursive: true, force: true })));
});
async function sandbox(): Promise<{ root: string; outside: string }> {
  const parent = await mkdtemp(join(tmpdir(), "rerail-replay-tools-test-"));
  directories.push(parent);
  const root = join(parent, "sandbox");
  const outside = join(parent, "outside");
  await Promise.all([mkdir(root), mkdir(outside)]);
  await writeFile(join(outside, "secret.json"), "{}");
  await writeFile(join(root, "run-input.json"), "{}");
  return { root, outside };
}

describe("sandboxFilePath", () => {
  it("accepts a file inside the sandbox by absolute or relative path", async () => {
    const { root } = await sandbox();
    const canonical = join(await realpath(root), "run-input.json");
    expect(await sandboxFilePath(root, join(root, "run-input.json"))).toBe(canonical);
    expect(await sandboxFilePath(root, "run-input.json")).toBe(canonical);
    expect(await sandboxFilePath(root, "nested/../run-input.json")).toBe(canonical);
  });
  it("rejects parent traversal", async () => {
    const { root } = await sandbox();
    await expect(sandboxFilePath(root, "../outside/secret.json")).rejects.toThrow("escapes sandbox");
    await expect(sandboxFilePath(root, join(root, "..", "outside", "secret.json"))).rejects.toThrow("escapes sandbox");
    await expect(sandboxFilePath(root, "a/../../outside/secret.json")).rejects.toThrow("escapes sandbox");
  });
  it("rejects absolute paths outside the sandbox", async () => {
    const { root, outside } = await sandbox();
    await expect(sandboxFilePath(root, join(outside, "secret.json"))).rejects.toThrow("escapes sandbox");
    await expect(sandboxFilePath(root, "/etc/passwd")).rejects.toThrow("escapes sandbox");
    await expect(sandboxFilePath(root, `${root}-sibling/secret.json`)).rejects.toThrow("escapes sandbox");
  });
  it("rejects the sandbox root itself and empty input", async () => {
    const { root } = await sandbox();
    await expect(sandboxFilePath(root, root)).rejects.toThrow("relative sandbox file path");
    await expect(sandboxFilePath(root, "")).rejects.toThrow("relative sandbox file path");
  });
  it("rejects links that escape the sandbox", async () => {
    const { root, outside } = await sandbox();
    await symlink(join(outside, "secret.json"), join(root, "leak.json"));
    await symlink(outside, join(root, "leak-dir"));
    await expect(sandboxFilePath(root, "leak.json")).rejects.toThrow("links and devices are forbidden");
    await expect(sandboxFilePath(root, "leak-dir/secret.json")).rejects.toThrow("escapes sandbox");
  });
});

describe("safePath", () => {
  it("rejects absolute, parent, and private runtime paths", async () => {
    const { root } = await sandbox();
    const canonical = await realpath(root);
    await expect(safePath(canonical, "/etc/passwd")).rejects.toThrow("relative sandbox file path");
    await expect(safePath(canonical, "../outside/secret.json")).rejects.toThrow("not accessible");
    await expect(safePath(canonical, "src/../../secret.json")).rejects.toThrow("not accessible");
    await expect(safePath(canonical, ".rerail/transcripts.sqlite")).rejects.toThrow("not accessible");
    await expect(safePath(canonical, ".git/config")).rejects.toThrow("not accessible");
    expect(await safePath(canonical, "run-input.json")).toBe(join(canonical, "run-input.json"));
  });
});

describe("replay glob", () => {
  it("matches globstars, braces and dotfiles without admitting unrelated fixtures", async () => {
    const { root } = await sandbox();
    await mkdir(join(root, "src"));
    await Promise.all([
      writeFile(join(root, ".hidden.ts"), ""),
      writeFile(join(root, "entry.ts"), ""),
      writeFile(join(root, "notes.md"), ""),
      writeFile(join(root, "src", "view.tsx"), ""),
      writeFile(join(root, "src", "ignore.txt"), ""),
    ]);
    const tools: ToolDefinition[] = [];
    const extension = await replayTools(root, false, {
      id: "glob", title: "Find source files", expectation: "match", explanation: "Inspect fixtures",
      ruleDigest: "fixture", ruleKind: "code", prompt: "Find source files", fixtures: [], steps: [],
    });
    await extension({
      registerTool(tool: ToolDefinition) { tools.push(tool); },
    } as unknown as ExtensionAPI);
    const glob = tools.find(tool => tool.name === "glob")!;
    const result = await glob.execute("glob", { pattern: "**/*.{ts,tsx}" }, undefined, undefined, {} as ExtensionContext);
    expect(result.content).toHaveLength(1);
    const content = result.content[0]!;
    expect(content.type).toBe("text");
    if (content.type !== "text") throw new Error("Glob did not return text");
    expect(content.text.split("\n").sort()).toEqual([".hidden.ts", "entry.ts", "src/view.tsx"]);
  });
});
