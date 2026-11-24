import { afterEach, describe, expect, it } from "vitest";
import { ASK, ASK_OPEN, LOG_NOTE, agentSeesHint, agentSeesPrompt, askHint, askPrompt, canDelete, deleteConsequences, fmtDay, libraryAttachments, splitPath } from "../../ui/render/rule-sheet-model.js";
import { createSuggestions } from "../../ui/views/create.js";
import { toStoredRules } from "../../ui/poll.js";
import { data } from "../../ui/state.js";
import { EDITED_TIP, EDIT_TITLE, changedParts, editedLine, pendingEditOf } from "../../ui/render/rule-edits.js";
import { approvalQueue, dismissApproved, noteWaiting, queuePosition, wasShownWaiting } from "../../ui/approval-queue.js";

const RULE = { id: "lib:no-raw-fetch", title: "No raw fetch", scope: "src/**/*.ts", mode: "Steer" };

const folderState = (folder: string, kind: string, id: string) => ({ folder, on: true, mode: "steer", source: { kind, id } });
const stored = (origin: "user" | "pack", states: object[]) => ({ origin, folderStates: states });

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
    [{ fromPack: false, log: false }, reaches],
    [{ fromPack: false, log: true }, LOG_NOTE],
    [{ fromPack: true, log: false }, `${copy} ${reaches}`],
    [{ fromPack: true, log: true }, `${copy} ${LOG_NOTE}`],
  ])("for %j says what saving does", (rule, hint) => {
    expect(agentSeesHint(rule)).toBe(hint);
  });
});

describe("the prompt box's hint", () => {
  it("says a change applies once saved, and first that a pack rule's edit is a copy", () => {
    expect(askHint({ fromPack: false })).toBe("Applies in open sessions once your agent saves it.");
    expect(askHint({ fromPack: true })).toBe("Saved as your copy. This one turns off. Applies in open sessions once your agent saves it.");
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
    expect(canDelete(stored("pack", [folderState("", "pack", "typescript")]))).toBe(false);
    expect(canDelete(stored("pack", [folderState("", "rerail_rule", "typescript:no-any")]))).toBe(false);
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

type Fields = Record<string, unknown>;
const DAY = 86_400_000;

/** One entry of `GET /api/repository/rules`; `rule` fields override the rule document's. */
const ruleEntry = (id: string, fields: Fields = {}, rule: Fields = {}) => ({
  kind: "code",
  rule: {
    id, mode: "steer", description: "No debugger statements.", scope: { include: ["src/**/*.ts"] },
    clauses: [{ match: { kind: "debugger_statement" }, violation: { message: "Remove the debugger statement." } }], ...rule,
  },
  folder: "", on: true, mode: "steer",
  source: { kind: "pack", id: "javascript" }, onSource: { by: "pack-start" }, modeSource: { by: "pack-start" },
  suppressions: "honour", suppressionsSource: { by: "rule-default" }, config: null, configSource: {}, configError: null,
  revision: 1, addedAt: Date.now(), updatedAt: Date.now(), copiedFromRuleId: null, approval: null,
  ...fields,
});

/** The coding agent's edit of `original`, saved as a library copy that waits for approval at `digest`. */
const copyOf = (original: string, digest: string, rule: Fields = {}, fields: Fields = {}) => ruleEntry(`${original}-copy`, {
  source: { kind: "library_rule", id: `${original}-copy`, packId: null }, modeSource: { by: "awaiting-approval" }, mode: "log",
  copiedFromRuleId: original, approval: { subject: { kind: "rule", id: `${original}-copy`, digest }, approved: false }, ...fields,
}, rule);

const loadRules = (...entries: ReturnType<typeof ruleEntry>[]) => { data.rules = toStoredRules(entries); };
const ruleById = (id: string) => data.rules.get(id)!;

describe("a rule a coding agent edited", () => {
  afterEach(() => {
    data.loaded = false;
    data.rules = new Map();
    data.activityAll = {};
  });

  describe("its sheet's notice and its row's Edited", () => {
    it("has a pending edit while its copy waits for approval, and none once the copy is approved", () => {
      loadRules(ruleEntry("no-debugger"), copyOf("no-debugger", "sha256:a", { description: "No debugger, ever." }));
      expect(pendingEditOf(ruleById("no-debugger"))?.rule.id).toBe("no-debugger-copy");
      // The copy itself is not an edited rule.
      expect(pendingEditOf(ruleById("no-debugger-copy"))).toBeNull();

      loadRules(ruleEntry("no-debugger"), copyOf("no-debugger", "sha256:a", {}, {
        approval: { subject: { kind: "rule", id: "no-debugger-copy", digest: "sha256:a" }, approved: true },
      }));
      expect(pendingEditOf(ruleById("no-debugger"))).toBeNull();
    });

    it("has none while the original itself waits for approval, whose notice its sheet shows instead", () => {
      loadRules(
        ruleEntry("no-debugger", { approval: { subject: { kind: "pack", id: "javascript", digest: "sha256:p" }, approved: false } }),
        copyOf("no-debugger", "sha256:k", { description: "No debugger, ever." }),
      );
      expect(pendingEditOf(ruleById("no-debugger"))).toBeNull();
    });

    it("names the parts that changed, in the sheet's order and by its section names", () => {
      loadRules(ruleEntry("no-debugger"), copyOf("no-debugger", "sha256:b", {
        scope: { include: ["src/**"] },
        clauses: [{ match: { kind: "debugger_statement" }, violation: { message: "Take the debugger out." } }],
      }));
      expect(changedParts(ruleById("no-debugger"), ruleById("no-debugger-copy"))).toEqual(["What the agent sees", "Where it applies"]);

      loadRules(ruleEntry("no-debugger"), copyOf("no-debugger", "sha256:c", {
        description: "Never ship a debugger.",
        clauses: [{ match: { kind: "debugger_statement", inside: "function" }, violation: { message: "Remove the debugger statement." } }],
      }));
      expect(changedParts(ruleById("no-debugger"), ruleById("no-debugger-copy"))).toEqual(["Description", "What it checks"]);
    });

    it("counts a change of mode, which a coding agent makes through the rule's document, as On a violation", () => {
      // A copy that waits runs in Log whatever it is set to: one set to steer says so in its mode's source.
      loadRules(ruleEntry("no-debugger"), copyOf("no-debugger", "sha256:d"));
      expect(changedParts(ruleById("no-debugger"), ruleById("no-debugger-copy"))).toEqual([]);

      // Steer to Log: the copy's own document says Log, so no approval holds its mode.
      loadRules(ruleEntry("no-debugger"), copyOf("no-debugger", "sha256:d", { mode: "log" }, { modeSource: { by: "rule-default" } }));
      expect(changedParts(ruleById("no-debugger"), ruleById("no-debugger-copy"))).toEqual(["On a violation"]);

      // Log to Steer, with new words: the original is on Log, the copy is held on Log until approved.
      loadRules(ruleEntry("no-debugger", { mode: "log" }), copyOf("no-debugger", "sha256:d", {
        clauses: [{ match: { kind: "debugger_statement" }, violation: { message: "Take the debugger out." } }],
      }));
      expect(changedParts(ruleById("no-debugger"), ruleById("no-debugger-copy"))).toEqual(["On a violation", "What the agent sees"]);
    });

    it("says the edit waits for approval and when it was made, and the row's hover says so too", () => {
      expect(EDIT_TITLE).toBe("An edit of this rule waits for approval.");
      expect(editedLine({ addedAt: Date.now() })).toBe("Edited today. This rule keeps running as it was until the edit is approved.");
      expect(editedLine({ addedAt: Date.now() - 3 * DAY })).toBe("Edited 3 days ago. This rule keeps running as it was until the edit is approved.");
      expect(editedLine({ addedAt: null })).toBe("This rule keeps running as it was until the edit is approved.");
      expect(EDITED_TIP).toBe("An edit of this rule waits for approval. Open it to review the edit.");
    });
  });

  describe("waiting with the rules your coding agent added", () => {
    const added = (id: string, title: string) => ruleEntry(id, {
      source: { kind: "library_rule", id, packId: null }, onSource: { by: "rule-default" }, modeSource: { by: "awaiting-approval" }, mode: "log",
      approval: { subject: { kind: "rule", id, digest: `sha256:${id}` }, approved: false },
    }, { title });
    const activity = (distinctViolations: number) => ({ measured: true, distinctViolations, loggedViolations: distinctViolations, rerails: 0, sessions: 0, lastSeenAt: null, audits: 1 });

    it("is in one queue with them, the one that caught the most first, then by title", () => {
      data.loaded = true;
      loadRules(ruleEntry("no-debugger"), copyOf("no-debugger", "sha256:l", { title: "No debugger" }), added("b-rule", "Beta"), added("a-rule", "Alpha"));
      expect(approvalQueue().map(stored => stored.rule.id)).toEqual(["a-rule", "b-rule", "no-debugger-copy"]);

      data.activityAll = { "no-debugger-copy": activity(2), "b-rule": activity(5) } as never;
      expect(approvalQueue().map(stored => stored.rule.id)).toEqual(["b-rule", "no-debugger-copy", "a-rule"]);
    });

    it("says where a rule stands among what waits, and nothing alone or outside it", () => {
      data.loaded = true;
      loadRules(ruleEntry("no-debugger"), copyOf("no-debugger", "sha256:m"), added("a-rule", "Alpha"));
      const queue = approvalQueue();

      expect(queuePosition(queue, "a-rule")).toBe("1 of 2 waiting");
      expect(queuePosition(queue, "no-debugger-copy")).toBe("2 of 2 waiting");
      expect(queuePosition(queue, "no-debugger")).toBeNull();
      expect(queuePosition(queue.slice(0, 1), "a-rule")).toBeNull();
    });

    it("stays pending until it is approved: nothing dismisses it", () => {
      loadRules(ruleEntry("no-debugger"), copyOf("no-debugger", "sha256:n", { description: "No debugger, ever." }));
      expect(pendingEditOf(ruleById("no-debugger"))?.approval?.subject.digest).toBe("sha256:n");
    });
  });

  describe("a rule approved in this visit", () => {
    it("is one its sheet showed waiting, until its Approved is dismissed", () => {
      expect(wasShownWaiting("a-rule")).toBe(false);

      noteWaiting("a-rule");
      expect(wasShownWaiting("a-rule")).toBe(true);

      dismissApproved("a-rule");
      expect(wasShownWaiting("a-rule")).toBe(false);
    });
  });
});
