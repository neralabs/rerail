import { afterEach, describe, expect, it, vi } from "vitest";
import { mkdir, mkdtemp, readdir, readFile, rm, writeFile } from "node:fs/promises";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { connectCodingAgents, forgetSeenCodingAgents, recordSeenCodingAgents, refreshCodingAgents, seenCodingAgents } from "../../src/setup/coding-agents.js";
import { pinnedNodePath } from "../../src/setup/node-path.js";
import { CODEX_HOOK_EVENTS, registerHarnesses } from "../../src/setup/harness-registration.js";

const temporaryDirectories: string[] = [];
afterEach(async () => {
  vi.unstubAllEnvs();
  await Promise.all(temporaryDirectories.splice(0).map(path => rm(path, { recursive: true, force: true })));
});

/** Pi's settings, after writing `{ packages }` when given, as Pi's gallery or set-up wrote them. */
async function piSettings(home: string, packages?: unknown[]): Promise<unknown> {
  const settings = join(home, ".pi", "agent", "settings.json");
  if (packages !== undefined) {
    await mkdir(dirname(settings), { recursive: true });
    await writeFile(settings, JSON.stringify({ packages }));
  }
  return JSON.parse(await readFile(settings, "utf8"));
}

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

  // `pi install npm:@neralabs/rerail` from Pi's gallery stands in for the managed install, so `rerail update` swaps it.
  it("swaps Pi's gallery entry for the managed install, and leaves it for any other root", async () => {
    const { base, root, home, packageRoot } = await fixture();
    vi.stubEnv("RERAIL_HOME", join(base, "rerail-home"));
    await piSettings(home, ["npm:@other/pkg", "npm:@neralabs/rerail"]);

    const checkout = await refreshCodingAgents(await packageRoot("checkout"), home);
    expect(checkout.agents).toEqual([]);
    expect(await piSettings(home)).toEqual({ packages: ["npm:@other/pkg", "npm:@neralabs/rerail"] });

    const managed = await refreshCodingAgents(root, home);
    expect(managed.agents).toEqual(["pi", "claude-code"]);
    expect(await piSettings(home)).toEqual({ packages: ["npm:@other/pkg", root] });
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

describe("connectCodingAgents", () => {
  it("connects the other agents when one's config is not plain JSON, and leaves that file as it is", async () => {
    const { root, home } = await fixture();
    const settings = join(home, ".claude", "settings.json");
    const commented = "{\n  // mine\n  \"theme\": \"dark\",\n}\n";
    await writeFile(settings, commented);
    await mkdir(join(home, ".cursor"));
    const result = await connectCodingAgents(root, home, ["claude-code", "cursor"]);
    expect(result.failures).toEqual([{ agent: "claude-code", file: settings, message: expect.stringContaining("JSON") }]);
    expect(result.changed).toEqual(["cursor"]);
    expect(result.all).toEqual(["cursor"]);
    expect(await readFile(settings, "utf8")).toBe(commented);
    expect(await readdir(join(home, ".claude"))).toEqual(expect.not.arrayContaining([expect.stringMatching(/\.bak$/u)]));
    expect(await readFile(join(home, ".cursor", "hooks.json"), "utf8")).toContain(join(root, "dist", "hosts", "cursor", "cursor-hook.js"));
  });

  it("counts an agent as connected when its registration is out of date, as after Node moved, without touching it", async () => {
    const { root, home } = await fixture();
    // Registered by a Node that has moved since (nvm switched, or Homebrew's versioned keg is gone).
    const settings = join(home, ".claude", "settings.json");
    const registered = await readFile(settings, "utf8");
    const stale = registered.replaceAll(pinnedNodePath(), "/old/node-v24.0.0/bin/node");
    expect(stale).not.toBe(registered);
    await writeFile(settings, stale);

    const result = await connectCodingAgents(root, home, []);
    expect(result.all).toEqual(["claude-code"]);
    expect(result.changed).toEqual([]);
    expect(await readFile(settings, "utf8")).toBe(stale);
  });

  it("swaps a Pi gallery entry added since Pi was connected, though Pi is not named, and does not count Pi as newly connected", async () => {
    const { base, root, home } = await fixture();
    vi.stubEnv("RERAIL_HOME", join(base, "rerail-home"));
    await piSettings(home, [root, "npm:@neralabs/rerail@0.1.0"]);

    const result = await connectCodingAgents(root, home, []);
    expect(result.changed).toEqual([]);
    expect(result.all).toEqual(["pi", "claude-code"]);
    expect(await piSettings(home)).toEqual({ packages: [root] });
  });

  it("says OMP loads its own plugin install of Rerail too whenever OMP is connected, named or not", async () => {
    const { root, home } = await fixture();
    await registerHarnesses({ root, home, harnesses: ["omp"] });
    const copy = join(home, ".omp", "plugins", "node_modules", "@neralabs", "rerail");
    await mkdir(copy, { recursive: true });
    await writeFile(join(copy, "package.json"), JSON.stringify({ name: "@neralabs/rerail", version: "0.1.0" }));

    expect((await connectCodingAgents(root, home, [])).ompPlugin).toBe(true);
    expect((await connectCodingAgents(root, home, ["omp"])).ompPlugin).toBe(true);
  });
});

describe("seen coding agents", () => {
  it("has no record until one is written, keeps every agent recorded since, and forgets one it could not connect", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-seen-"));
    temporaryDirectories.push(home);
    expect(seenCodingAgents(home)).toBeUndefined();
    await recordSeenCodingAgents(home, []);
    expect(seenCodingAgents(home)).toEqual([]);
    await recordSeenCodingAgents(home, ["cursor"]);
    await recordSeenCodingAgents(home, ["codex", "cursor"]);
    expect(seenCodingAgents(home)).toEqual(["codex", "cursor"]);
    await forgetSeenCodingAgents(home, ["codex"]);
    expect(seenCodingAgents(home)).toEqual(["cursor"]);
  });

  it("treats a record it cannot read as no record", async () => {
    const home = await mkdtemp(join(tmpdir(), "rerail-seen-"));
    temporaryDirectories.push(home);
    await writeFile(join(home, "coding-agents.json"), "{ not json");
    expect(seenCodingAgents(home)).toBeUndefined();
    await recordSeenCodingAgents(home, ["claude-code"]);
    expect(seenCodingAgents(home)).toEqual(["claude-code"]);
  });
});
