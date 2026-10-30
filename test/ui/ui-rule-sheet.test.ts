import { describe, expect, it } from "vitest";
import { ASK, ASK_OPEN, LOG_NOTE, agentSeesHint, agentSeesPrompt, askPrompt, canDelete, deleteConsequences, fmtDay, libraryAttachments, splitPath } from "../../ui/render/rule-sheet-model.js";
import { createSuggestions } from "../../ui/views/create.js";

const RULE = { id: "lib:no-raw-fetch", title: "No raw fetch", scope: "src/**/*.ts", mode: "Steer" };

const folderState = (folder: string, kind: string, id: string) => ({ folder, on: true, mode: "steer", source: { kind, id } });
const stored = (origin: "user" | "rerail", states: object[]) => ({ origin, folderStates: states });

describe("the Edit with coding agent prompts", () => {
  it("offers one prompt per chip, each asking Rerail about the rule by id", () => {
    const prompts = ASK.map(([intent]) => askPrompt(RULE, intent));
    expect(new Set(prompts).size).toBe(ASK.length);
    for (const prompt of prompts) {
      expect(prompt.startsWith("Using Rerail, ")).toBe(true);
      expect(prompt).toContain(`"${RULE.id}"`);
    }
  });

  it("leaves the change prompts open for the person to finish, and closes the question", () => {
    for (const [intent] of ASK) {
      const prompt = askPrompt(RULE, intent);
      expect(prompt.endsWith(": ")).toBe(ASK_OPEN[intent] === true);
    }
  });

  it("states the current scope and mode in the prompts that change them", () => {
    expect(askPrompt(RULE, "scope")).toContain("Currently it applies to: src/**/*.ts. Show me");
    expect(askPrompt({ ...RULE, scope: "Every file in the repository." }, "scope")).toContain("repository. Show me");
    expect(askPrompt(RULE, "delivery")).toContain("Currently, on a violation: Steer.");
  });

  it("hands the edited agent text to the coding agent verbatim, trimmed", () => {
    const prompt = agentSeesPrompt(RULE.id, "  Use the api client instead.\n");
    expect(prompt.startsWith(`Using Rerail, update the rule "${RULE.id}"`)).toBe(true);
    expect(prompt.endsWith("to: Use the api client instead.")).toBe(true);
  });
});

describe("the What the agent sees hint", () => {
  const reaches = "Reaches open sessions once your agent saves it.";
  const copy = "Saved as your copy. This one turns off.";

  it.each([
    [{ byRerail: false, log: false }, reaches],
    [{ byRerail: false, log: true }, LOG_NOTE],
    [{ byRerail: true, log: false }, `${copy} ${reaches}`],
    [{ byRerail: true, log: true }, `${copy} ${LOG_NOTE}`],
  ])("for %j says what saving does", (rule, hint) => {
    expect(agentSeesHint(rule)).toBe(hint);
  });
});

describe("dates on the sheet", () => {
  it("show the year", () => {
    expect(fmtDay(Date.UTC(2026, 8, 3, 12))).toBe("Sep 3, 2026");
    expect(fmtDay(Date.UTC(2025, 11, 31, 12))).toBe("Dec 31, 2025");
  });

  it("read as a dash when the daemon has none", () => {
    expect(fmtDay(null)).toBe("–");
    expect(fmtDay(undefined)).toBe("–");
  });
});

describe("Delete on the sheet", () => {
  it("is offered for a rule created by you that is added here on its own", () => {
    expect(canDelete(stored("user", [folderState("", "library_rule", "lib:no-raw-fetch")]))).toBe(true);
  });

  it("is not offered for a rule that comes from a pack", () => {
    expect(canDelete(stored("rerail", [folderState("", "pack", "typescript")]))).toBe(false);
    expect(canDelete(stored("rerail", [folderState("", "rerail_rule", "typescript:no-any")]))).toBe(false);
  });

  it("removes the rule at every folder it is added at, and only its library attachments", () => {
    const rule = stored("user", [
      folderState("", "library_rule", "lib:x"),
      folderState("packages/web", "library_rule", "lib:x"),
      folderState("packages/api", "pack", "typescript"),
    ]);
    expect(libraryAttachments(rule)).toEqual([
      { sourceKind: "library_rule", sourceId: "lib:x", folder: "" },
      { sourceKind: "library_rule", sourceId: "lib:x", folder: "packages/web" },
    ]);
  });

  it("says what happens to past violations only when there are some, and suggests Off only for a rule that is on", () => {
    const none = deleteConsequences({ repo: "storefront", pastViolations: 0, on: false });
    const one = deleteConsequences({ repo: "storefront", pastViolations: 1, on: true });
    const many = deleteConsequences({ repo: "storefront", pastViolations: 12, on: true });
    expect(none).toHaveLength(2);
    expect(one).toHaveLength(4);
    expect(one.some(line => line.includes("1 past violation stays"))).toBe(true);
    expect(many.some(line => line.includes("12 past violations stay"))).toBe(true);
    expect(none.every(line => !line.includes("turn it off"))).toBe(true);
  });
});

describe("a violation's location", () => {
  it.each([
    ["src/api/client.ts", { file: "client.ts", dir: "src/api" }],
    ["README.md", { file: "README.md", dir: "" }],
    [null, { file: "", dir: "" }],
  ])("%s splits into file and folder", (path, parts) => {
    expect(splitPath(path)).toEqual(parts);
  });
});

describe("Places to start on Create a rule", () => {
  const packs = [{ id: "typescript", name: "TypeScript" }, { id: "react", name: "React" }];
  const recommendation = (packId: string) => ({ packId, folder: "", reason: "fits" });

  it("suggests only recommended packs that are not added", () => {
    const attached = new Map([["typescript", [""]]]);
    expect(createSuggestions([recommendation("typescript"), recommendation("react")], attached, packs))
      .toEqual([{ packId: "react", name: "React", reason: "fits" }]);
  });

  it("is empty before recommendations arrive, and skips packs it does not know or repeats", () => {
    expect(createSuggestions(null, new Map(), packs)).toEqual([]);
    expect(createSuggestions([recommendation("unknown"), recommendation("react"), { ...recommendation("react"), folder: "web" }], new Map(), packs))
      .toEqual([{ packId: "react", name: "React", reason: "fits" }]);
  });
});
