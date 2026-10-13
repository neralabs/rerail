import { describe, expect, it } from "vitest";
import {
  headlineText,
  linkSteers,
  sessionStartNote,
  steerBlock,
  steerHeadlines,
} from "../../../src/engine/code-rule/rerail-steer.js";
import { steerOrigin } from "../../../src/engine/code-rule/steer-origin.js";

const DASHBOARD = "http://127.0.0.1:4791/";
const STEER = { mode: "steer" } as const;

describe("Rerail steer text", () => {
  it("links every steer to its rule on the dashboard, and leaves the links relative without one", () => {
    const content = `${steerBlock("ts:no-any", "Use unknown.", STEER)}\n\n${steerBlock("acme:no-print", "Remove print.\n  Suggested fix: delete 1:1-1:4", STEER)}`;
    expect(linkSteers(content, DASHBOARD)).toBe([
      "◆ Rerail · ts:no-any  steered",
      "Set to Steer: fix it, or tell the person in one line why you won't.",
      "Use unknown.",
      "Why? → http://127.0.0.1:4791/#rule/ts%3Ano-any",
      "",
      "◆ Rerail · acme:no-print  steered",
      "Set to Steer: fix it, or tell the person in one line why you won't.",
      "Remove print.",
      "  Suggested fix: delete 1:1-1:4",
      "Why? → http://127.0.0.1:4791/#rule/acme%3Ano-print",
    ].join("\n"));
    expect(linkSteers(content, undefined)).toBe(content);
  });

  it("tells the coding agent the rule's pack and mode, and steers a Block-mode rule until blocking ships", () => {
    expect(steerBlock("complexity:cyclomatic-python", "Too complex.", { mode: "block", origin: steerOrigin("complexity:cyclomatic-python") }))
      .toBe([
        "◆ Rerail · complexity:cyclomatic-python  steered",
        "From the Cyclomatic complexity (limit 6) rule pack. Set to Block, which steers until blocking ships: fix it, or tell the person in one line why you won't.",
        "Too complex.",
        "Why? → #rule/complexity%3Acyclomatic-python",
      ].join("\n"));
    expect(steerBlock("x", "Noted.", { mode: "log" }).split("\n").slice(0, 2)).toEqual(["○ Rerail · x  logged", "Set to Log: recorded only."]);
  });

  it("names where a rule comes from: its pack, the pack's id when the pack is not loaded, or the library", () => {
    expect(steerOrigin("complexity:cyclomatic-python")).toBe("From the Cyclomatic complexity (limit 6) rule pack.");
    expect(steerOrigin("typescript-eslint:no-explicit-any", { kind: "pack", packId: "typescript" })).toBe("From the typescript rule pack.");
    expect(steerOrigin("mine", { kind: "library_rule", packId: null })).toBe("From your rule library.");
    expect(steerOrigin("local:unknown")).toBeUndefined();
  });

  it("gives the person one line per steered rule, with its link once the link is absolute", () => {
    const content = `${steerBlock("a", "One.\nTwo.", STEER)}\n\n${steerBlock("b", "Three.", STEER)}`;
    expect(steerHeadlines(linkSteers(content, DASHBOARD))).toEqual([
      { header: "◆ Rerail · a  steered", url: "http://127.0.0.1:4791/#rule/a" },
      { header: "◆ Rerail · b  steered", url: "http://127.0.0.1:4791/#rule/b" },
    ]);
    expect(steerHeadlines(content).map(headline => headline.url)).toEqual([undefined, undefined]);
  });

  it("puts the why link on the word where the agent can draw one, and prints it after the word where it cannot", () => {
    const headline = { header: "◆ Rerail · a  steered", url: "http://127.0.0.1:4791/#rule/a" };
    expect(headlineText(headline, "hyperlink")).toBe("◆ Rerail · a  steered  ·  \u001b]8;;http://127.0.0.1:4791/#rule/a\u0007why\u001b]8;;\u0007");
    expect(headlineText(headline, "markdown")).toBe("◆ Rerail · a  steered  ·  [why](http://127.0.0.1:4791/#rule/a)");
    expect(headlineText(headline, "plain")).toBe("◆ Rerail · a  steered  ·  why → http://127.0.0.1:4791/#rule/a");
    expect(headlineText({ ...headline, url: undefined }, "hyperlink")).toBe("◆ Rerail · a  steered");
  });

  it("keeps terminal control sequences in a rule id out of the person's line", () => {
    const [headline] = steerHeadlines(linkSteers(steerBlock("x\u001b[2Jy", "Text.", STEER), DASHBOARD));
    expect(headlineText(headline!, "plain")).toBe("◆ Rerail · x[2Jy  steered  ·  why → http://127.0.0.1:4791/#rule/x%1B%5B2Jy");
  });

  it("finds no headline in guidance that is not a Rerail steer, even when it mentions one", () => {
    expect(steerHeadlines("Ask before force-pushing.\nThe old text said ◆ Rerail · x  steered inline.")).toEqual([]);
  });

  it("tells the coding agent how to treat a steer, how to answer why, and to ask before turning a rule off", () => {
    const note = sessionStartNote(DASHBOARD, "codex", "thread 1", { setUp: true, ruleCount: 26 });
    expect(note).toContain("Rerail is on in this repo: 26 rules are on.");
    expect(note).toContain("message starting \"◆ Rerail\"");
    expect(note).toContain("if you decide not to follow it, tell the person in one line why");
    expect(note).toContain("ask the person: Off, Log (kept on record, you are not told), or leave it.");
    expect(note).toContain("$rerail-stop is the person's own command");
    expect(note).toContain("http://127.0.0.1:4791/?sessionId=thread+1&host=codex#sessions");
    expect(note).not.toContain("Cursor shows the person nothing");
    expect(sessionStartNote(undefined, "cursor", "s", { setUp: true, ruleCount: null })).toContain("Cursor shows the person nothing from Rerail.");
    expect(sessionStartNote(undefined, "claude-code", "s", { setUp: true, ruleCount: null })).not.toContain("dashboard:");
    expect(sessionStartNote(undefined, "claude-code", "s", { setUp: true, ruleCount: null })).toContain("Rerail is on in this repo. It checks");
  });

  it("tells the coding agent that a repository that is not set up checks no rules, and how the person sets it up", () => {
    const note = sessionStartNote(DASHBOARD, "codex", "thread 1", { setUp: false });
    expect(note).toContain("not set up yet, so no rules are checked");
    expect(note).toContain("$rerail-start");
    expect(note).not.toContain("◆ Rerail");
  });
});
