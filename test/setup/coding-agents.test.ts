import { afterEach, describe, expect, it } from "vitest";
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { refreshCodingAgents } from "../../src/setup/coding-agents.js";
import { CODEX_HOOK_EVENTS, registerHarnesses } from "../../src/setup/harness-registration.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

/** A Rerail package root and a coding-agent home with Claude Code registered against it, `/rerail-show-me` since deleted. */
async function fixture() {
  const base = await mkdtemp(join(tmpdir(), "rerail-coding-agents-"));
  temporaryDirectories.push(base);
  const packageRoot = async (name: string): Promise<string> => {
    const root = join(base, name, "current");
    await mkdir(join(root, "dist"), { recursive: true });
    await writeFile(join(root, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "1.0.0" }));
    return root;
  };
  const root = await packageRoot("rerail-home");
  const home = join(base, "home");
  await registerHarnesses({ root, home, harnesses: ["claude-code"] });
  const commands = join(home, ".claude", "commands");
  await rm(join(commands, "rerail-show-me.md"));
  return { base, root, home, commands, packageRoot };
}

describe("refreshCodingAgents", () => {
  it("installs new commands and removes retired ones in registered agents only, then has nothing left to do", async () => {
    const { root, home, commands, packageRoot } = await fixture();
    const retired = join(commands, "rerail-ui.md");
    await writeFile(retired, `Run:\n${process.execPath} ${join(root, "dist", "cli", "cli.js")} ui\n`);
    await mkdir(join(home, ".cursor"));
    const elsewhere = await packageRoot("elsewhere");
    await registerHarnesses({ root: elsewhere, home, harnesses: ["codex"] });
    const codexHooks = await readFile(join(home, ".codex", "hooks.json"), "utf8");

    const first = await refreshCodingAgents(root, home);
    expect(first.agents).toEqual(["claude-code"]);
    expect(first.failures).toEqual([]);
    expect(first.changes.filter(change => change.action !== "unchanged").map(change => change.path).sort())
      .toEqual([join(commands, "rerail-show-me.md"), retired].sort());
    expect(await readFile(join(commands, "rerail-show-me.md"), "utf8")).toContain(join(root, "dist", "cli", "cli.js"));
    expect((await readdir(commands)).filter(name => name.endsWith(".md"))).not.toContain("rerail-ui.md");
    expect(await readdir(join(home, ".cursor"))).toEqual([]);
    expect(await readFile(join(home, ".codex", "hooks.json"), "utf8")).toBe(codexHooks);

    const second = await refreshCodingAgents(root, home);
    expect(second.agents).toEqual(["claude-code"]);
    expect(second.changes.every(change => change.action === "unchanged")).toBe(true);
  });

  it("refreshes the other agents when a connected agent's registration cannot be written, and names that one", async () => {
    const { root, home, commands } = await fixture();
    await registerHarnesses({ root, home, harnesses: ["codex"] });
    const path = join(home, ".codex", "hooks.json");
    const config = JSON.parse(await readFile(path, "utf8")) as { hooks: Record<string, unknown> };
    // Rerail's other hooks still name the root, so Codex is connected; registering cannot add to an event that is not a list.
    config.hooks[CODEX_HOOK_EVENTS[0]!] = "not a list";
    await writeFile(path, JSON.stringify(config));

    const result = await refreshCodingAgents(root, home);
    expect(result.agents).toEqual(["claude-code"]);
    expect(result.failures).toEqual([{ agent: "codex", message: expect.stringContaining(path) }]);
    expect(result.unchecked).toEqual([]);
    expect(await readdir(commands)).toContain("rerail-show-me.md");
  });

  it("leaves an agent alone, without failing, when its configuration cannot be read to tell whether it is connected", async () => {
    const { root, home, commands } = await fixture();
    const cursorHooks = join(home, ".cursor", "hooks.json");
    await mkdir(join(home, ".cursor"));
    await writeFile(cursorHooks, "{ not json");

    const result = await refreshCodingAgents(root, home);
    expect(result.agents).toEqual(["claude-code"]);
    expect(result.failures).toEqual([]);
    expect(result.unchecked).toEqual([{ agent: "cursor", message: expect.stringContaining(cursorHooks) }]);
    expect(await readdir(join(home, ".cursor"))).toEqual(["hooks.json"]);
    expect(await readFile(cursorHooks, "utf8")).toBe("{ not json");
    expect(await readdir(commands)).toContain("rerail-show-me.md");
  });

  it("leaves OMP alone when its rerail MCP bridge names another install", async () => {
    const { root, home, packageRoot } = await fixture();
    await registerHarnesses({ root: await packageRoot("elsewhere"), home, harnesses: ["omp"] });
    const mcp = await readFile(join(home, ".omp", "agent", "mcp.json"), "utf8");

    const result = await refreshCodingAgents(root, home);
    expect(result.agents).toEqual(["claude-code"]);
    expect(await readFile(join(home, ".omp", "agent", "mcp.json"), "utf8")).toBe(mcp);
  });

  it("does nothing when no agent is registered", async () => {
    const base = await mkdtemp(join(tmpdir(), "rerail-coding-agents-"));
    temporaryDirectories.push(base);
    const home = join(base, "home");
    await mkdir(join(home, ".claude"), { recursive: true });
    await mkdir(join(home, ".cursor"));
    const result = await refreshCodingAgents(join(base, "root"), home);
    expect(result).toEqual({ agents: [], changes: [], notes: [], failures: [], unchecked: [] });
    expect((await readdir(home)).sort()).toEqual([".claude", ".cursor"]);
    expect(await readdir(join(home, ".cursor"))).toEqual([]);
  });
});
