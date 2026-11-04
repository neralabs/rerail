import { describe, expect, it } from "vitest";
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
