import { describe, expect, it } from "vitest";
import { ADAPTER_CAPABILITIES } from "../../src/engine/index.js";
import { commandFileText } from "../../src/harness/rerail-command-files.js";
import { RERAIL_COMMANDS, rerailCommandList } from "../../src/harness/rerail-commands.js";

const TERMINAL_LINE = "In a terminal: rerail status · rerail version · rerail update · rerail daemon stop";

describe("the /rerail command list", () => {
  it("ends by naming the terminal commands, for whoever looks for rerail outside the coding agent", () => {
    expect(rerailCommandList("/").split("\n").slice(-3)).toEqual(["Or just ask me, in plain words.", "", TERMINAL_LINE]);
  });

  it("names the terminal commands the same way for Codex, whose own commands are $-prefixed", () => {
    const list = rerailCommandList("$");
    expect(list).toContain("$rerail-manage");
    expect(list.split("\n").at(-1)).toBe(TERMINAL_LINE);
  });
});

describe("the command catalog", () => {
  // A Codex skill and a Claude Code command carry the description as a plain YAML value, where ": " starts a mapping.
  it.each(RERAIL_COMMANDS.map(command => [command.name, command.description]))("keeps %s's description valid in front matter", (_name, description) => {
    expect(description).not.toContain(": ");
    expect(description).not.toContain(" #");
  });
});

describe("the shell a command file is written for", () => {
  // `powershellCli` is set on Windows only; the agent's capability profile decides whether its files use it.
  const context = { cli: "'node' 'cli.js'", packageRoot: "/package", setupLauncher: "'rerail'", powershellCli: "& 'node' 'cli.js'" };

  it.each(["claude-code", "cursor", "codex"] as const)("follows the Windows shell in %s's capability profile", agent => {
    const start = commandFileText(agent, "rerail-start", context);
    const powershell = ADAPTER_CAPABILITIES[agent].windowsCommandShell === "powershell";
    expect(start.includes("& 'node' 'cli.js' start --relay --powershell\n")).toBe(powershell);
    expect(start.includes("\n'node' 'cli.js' start --relay\n")).toBe(!powershell);
  });

  it.each(["claude-code", "cursor", "codex"] as const)("fences %s's launcher lines in the language of the shell they run in", agent => {
    const language = ADAPTER_CAPABILITIES[agent].windowsCommandShell === "powershell" ? "powershell" : "sh";
    for (const name of RERAIL_COMMANDS.map(entry => entry.name)) {
      // A ```text block shows the person words, not a command.
      const fences = [...commandFileText(agent, name, context).matchAll(/^```(\w+)$/gmu)].map(match => match[1]).filter(fence => fence !== "text");
      expect(fences.length).toBeGreaterThan(0);
      expect(new Set(fences)).toEqual(new Set([language]));
    }
  });
});

describe("/rerail-show-me's write", () => {
  const context = { cli: "'node' 'cli.js'", packageRoot: "/package", setupLauncher: "'rerail'", powershellCli: "& 'node' 'cli.js'" };

  // Codex's hooks show its apply_patch from Codex 0.124, so its demo is written as every other agent's is.
  it.each(["claude-code", "cursor", "codex"] as const)("has %s write the demo file with its own edit tool", agent => {
    const text = commandFileText(agent, "rerail-show-me", context);
    expect(text).toContain("with your file edit or write tool");
    expect(text).not.toMatch(/shell command|<<'EOF'|Set-Content/u);
  });
});
