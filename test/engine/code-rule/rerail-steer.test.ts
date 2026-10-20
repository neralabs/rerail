import { describe, expect, it } from "vitest";
import {
  asLateSteer,
  headlineText,
  sessionStartNote,
  steerBlock,
  steerHeadlines,
  steerRules,
  type SteerFinding,
} from "../../../src/engine/code-rule/rerail-steer.js";
import { steerOrigin } from "../../../src/engine/code-rule/steer-origin.js";
import { installPack } from "../../support/installed-packs.js";

// The steer names a rule's pack from the installed catalog.
installPack("complexity");

const DASHBOARD = "http://127.0.0.1:4791/";
const finding = (overrides: Partial<SteerFinding> = {}): SteerFinding => ({
  ruleId: "ts:no-any", mode: "steer", message: "Use unknown.", location: "src/a.ts:1:2", ...overrides,
});

describe("Rerail steer text", () => {
  it("leads each trigger as the agent should read it, counting an edit's and an audit's problems", () => {
    const lead = (trigger: Parameters<typeof steerBlock>[0], count = 1) =>
      steerBlock(trigger, Array.from({ length: count }, () => finding())).split("\n")[0];
    expect(lead({ kind: "edit" })).toBe("Rerail found 1 problem in your last edit:");
    expect(lead({ kind: "edit" }, 2)).toBe("Rerail found 2 problems in your last edit:");
    expect(lead({ kind: "resteer" })).toBe("Rerail re-checked; still open:");
    expect(lead({ kind: "late" })).toBe("Rerail finished checking your earlier edits after you moved on:");
    expect(lead({ kind: "audit" })).toBe("Rerail checked the repository and found 1 problem:");
    expect(lead({ kind: "audit" }, 3)).toBe("Rerail checked the repository and found 3 problems:");
    expect(lead({ kind: "tool", tool: "Bash" })).toBe("Rerail has guidance for this Bash call:");
    expect(lead({ kind: "tool" })).toBe("Rerail has guidance for this step:");
  });

  it("ends with the agent's latitude: fix (or follow) it or these, or say why not; a Log is recorded only", () => {
    const latitude = (trigger: Parameters<typeof steerBlock>[0], findings: SteerFinding[]) => steerBlock(trigger, findings).split("\n").at(-1);
    expect(latitude({ kind: "edit" }, [finding()])).toBe("Fix it, or tell the user in one line why you won't.");
    expect(latitude({ kind: "edit" }, [finding(), finding()])).toBe("Fix these, or tell the user in one line why you won't.");
    // Blocking is not built yet, so a Block-mode rule steers.
    expect(latitude({ kind: "edit" }, [finding({ mode: "block" })])).toBe("Fix it, or tell the user in one line why you won't.");
    expect(latitude({ kind: "tool" }, [finding()])).toBe("Follow it, or tell the user in one line why you won't.");
    expect(latitude({ kind: "tool" }, [finding(), finding()])).toBe("Follow these, or tell the user in one line why you won't.");
    expect(latitude({ kind: "edit" }, [finding({ mode: "log" })])).toBe("Recorded only.");
  });

  it("writes a tool call's guidance as one finding with no place, its later lines under it", () => {
    expect(steerBlock({ kind: "tool", tool: "Bash" }, [finding({ ruleId: "no-force-push", message: "Ask before force-pushing.\nIt rewrites history.", location: undefined })]))
      .toBe([
        "Rerail has guidance for this Bash call:",
        "- Ask before force-pushing. [no-force-push]",
        "  It rewrites history.",
        "Follow it, or tell the user in one line why you won't.",
      ].join("\n"));
  });

  it("gives an edit's or a re-check's steer the late lead, and leaves any other steer as it is", () => {
    const edit = steerBlock({ kind: "edit" }, [finding(), finding()]);
    expect(asLateSteer(edit)).toBe(edit.replace("Rerail found 2 problems in your last edit:", "Rerail finished checking your earlier edits after you moved on:"));
    expect(asLateSteer(steerBlock({ kind: "resteer" }, [finding()])).split("\n")[0]).toBe("Rerail finished checking your earlier edits after you moved on:");
    const tool = steerBlock({ kind: "tool" }, [finding()]);
    expect(asLateSteer(tool)).toBe(tool);
    expect(asLateSteer("Rerail found 1 problem in your last edit: and more")).toBe("Rerail found 1 problem in your last edit: and more");
  });

  it("names where a rule comes from: its pack, the pack's id when the pack is not loaded, or the library", () => {
    expect(steerOrigin("complexity-cyclomatic-python")).toBe("From the Cyclomatic complexity (limit 6) rule pack.");
    expect(steerOrigin("typescript-eslint-no-explicit-any", { kind: "pack", packId: "typescript" })).toBe("From the typescript rule pack.");
    expect(steerOrigin("mine", { kind: "library_rule", packId: null })).toBe("From your rule library.");
    expect(steerOrigin("local:unknown")).toBeUndefined();
  });

  it("keeps each rule once, first-seen, with its state and source, out of the agent's text", () => {
    const findings = [finding({ ruleId: "complexity-cyclomatic-python" }), finding({ ruleId: "x", mode: "log" }), finding({ ruleId: "complexity-cyclomatic-python" })];
    expect(steerRules(findings, steerOrigin)).toEqual([
      { id: "complexity-cyclomatic-python", state: "steered", origin: "From the Cyclomatic complexity (limit 6) rule pack." },
      { id: "x", state: "logged" },
    ]);
    expect(steerBlock({ kind: "edit" }, findings)).not.toContain("rule pack");
  });

  it("gives the person one line per rule, the same line the steer header made, with its link once the rule has one", () => {
    expect(steerHeadlines([
      { id: "a", state: "steered", url: `${DASHBOARD}#rule/a` },
      { id: "b", state: "logged" },
    ])).toEqual([
      { header: "◆ Rerail · a  steered", url: "http://127.0.0.1:4791/#rule/a" },
      { header: "○ Rerail · b  logged", url: undefined },
    ]);
  });

  it("puts the why? link on the word where the agent can draw one, and prints it after the word where it cannot", () => {
    const headline = { header: "◆ Rerail · a  steered", url: "http://127.0.0.1:4791/#rule/a" };
    expect(headlineText(headline, "hyperlink")).toBe("◆ Rerail · a  steered  ·  \u001b]8;;http://127.0.0.1:4791/#rule/a\u0007why?\u001b]8;;\u0007");
    expect(headlineText(headline, "markdown")).toBe("◆ Rerail · a  steered  ·  [why?](http://127.0.0.1:4791/#rule/a)");
    expect(headlineText(headline, "plain")).toBe("◆ Rerail · a  steered  ·  why? → http://127.0.0.1:4791/#rule/a");
    expect(headlineText({ ...headline, url: undefined }, "hyperlink")).toBe("◆ Rerail · a  steered");
    expect(headlineText(headline, "plain", "<mark>")).toBe("◆ <mark> · a  steered  ·  why? → http://127.0.0.1:4791/#rule/a");
  });

  it("keeps terminal control sequences in a rule id out of the person's line", () => {
    const [headline] = steerHeadlines([{ id: "x\u001b[2Jy", state: "steered", url: `${DASHBOARD}#rule/${encodeURIComponent("x\u001b[2Jy")}` }]);
    expect(headlineText(headline!, "plain")).toBe("◆ Rerail · x[2Jy  steered  ·  why? → http://127.0.0.1:4791/#rule/x%1B%5B2Jy");
  });

  it("tells the coding agent how to treat a steer, how to answer why, and to ask before turning a rule off", () => {
    const note = sessionStartNote(DASHBOARD, "codex", "thread 1", { setUp: true, ruleCount: 26 });
    expect(note).toContain("Rerail is on in this repo: 26 rules are on.");
    expect(note).toContain("message starting \"Rerail found\" that lists each problem with its location, what to do, and the rule id in brackets.");
    expect(note).toContain("if you decide not to follow it, tell the user in one line why");
    expect(note).toContain("answer from that message: the rule id and what it matched. The user can open the rule from the why? link on their own line.");
    expect(note).toContain("name the rule and ask the user: Off, Log (kept on record, you are not told), or leave it.");
    expect(note).toContain("$rerail-stop is the user's own command");
    expect(note).not.toMatch(/the person|◆|rule pack/u);
    expect(note).toContain("http://127.0.0.1:4791/?sessionId=thread+1&host=codex#sessions");
    expect(note).not.toContain("Cursor shows the user nothing");
    expect(sessionStartNote(undefined, "cursor", "s", { setUp: true, ruleCount: null })).toContain("Cursor shows the user nothing from Rerail.");
    expect(sessionStartNote(undefined, "claude-code", "s", { setUp: true, ruleCount: null })).not.toContain("dashboard:");
    expect(sessionStartNote(undefined, "claude-code", "s", { setUp: true, ruleCount: null })).toContain("Rerail is on in this repo. It checks");
  });

  it("tells the coding agent that a repository that is not set up checks no rules, and how the user sets it up", () => {
    const note = sessionStartNote(DASHBOARD, "codex", "thread 1", { setUp: false });
    expect(note).toContain("not set up yet, so no rules are checked");
    expect(note).toContain("$rerail-start");
    expect(note).not.toContain("◆ Rerail");
  });
});
