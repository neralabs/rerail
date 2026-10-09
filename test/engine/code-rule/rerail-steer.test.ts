import { describe, expect, it } from "vitest";
import {
  linkSteers,
  sessionStartNote,
  steerBlock,
  steerHeadlines,
} from "../../../src/engine/code-rule/rerail-steer.js";

const DASHBOARD = "http://127.0.0.1:4791/";

describe("Rerail steer text", () => {
  it("links every steer to its rule on the dashboard, and leaves the links relative without one", () => {
    const content = `${steerBlock("ts:no-any", "Use unknown.")}\n\n${steerBlock("acme:no-print", "Remove print.\n  Suggested fix: delete 1:1-1:4")}`;
    expect(linkSteers(content, DASHBOARD)).toBe([
      "▲ Rerail · ts:no-any  Steered.",
      "Use unknown.",
      "Why? → http://127.0.0.1:4791/#rule/ts%3Ano-any",
      "",
      "▲ Rerail · acme:no-print  Steered.",
      "Remove print.",
      "  Suggested fix: delete 1:1-1:4",
      "Why? → http://127.0.0.1:4791/#rule/acme%3Ano-print",
    ].join("\n"));
    expect(linkSteers(content, undefined)).toBe(content);
  });

  it("gives the person one line per steered rule, with its link", () => {
    const content = linkSteers(`${steerBlock("a", "One.\nTwo.")}\n\n${steerBlock("b", "Three.")}`, DASHBOARD);
    expect(steerHeadlines(content)).toEqual([
      "▲ Rerail · a  Steered. Why? → http://127.0.0.1:4791/#rule/a",
      "▲ Rerail · b  Steered. Why? → http://127.0.0.1:4791/#rule/b",
    ]);
  });

  it("finds no headline in guidance that is not a Rerail steer, even when it mentions one", () => {
    expect(steerHeadlines("Ask before force-pushing.\nThe old text said ▲ Rerail · x  Steered. inline.")).toEqual([]);
  });

  it("tells the coding agent where its session page is, and which command lists the rest", () => {
    const note = sessionStartNote(DASHBOARD, "codex", "thread 1", { setUp: true, ruleCount: 26 });
    expect(note).toContain("Rerail is on in this repo: 26 rules are on.");
    expect(note).toContain("http://127.0.0.1:4791/?sessionId=thread+1&host=codex#sessions");
    expect(note).toContain("$rerail");
    expect(sessionStartNote(undefined, "claude-code", "s", { setUp: true, ruleCount: null })).not.toContain("dashboard:");
    expect(sessionStartNote(undefined, "claude-code", "s", { setUp: true, ruleCount: null })).toContain("Rerail is on in this repo. It checks");
  });

  it("tells the coding agent that a repository that is not set up checks no rules, and how the person sets it up", () => {
    const note = sessionStartNote(DASHBOARD, "codex", "thread 1", { setUp: false });
    expect(note).toContain("not set up yet, so no rules are checked");
    expect(note).toContain("$rerail-start");
    expect(note).not.toContain("▲ Rerail");
  });
});
