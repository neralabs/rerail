import { describe, expect, it, vi } from "vitest";
import { guardCompaction, RERAIL_COMPACTION_LABEL, type CompactionPreparationLike } from "../../src/harness/compaction-guard.js";

type Message = { role: string; timestamp: number; [field: string]: unknown };

const usage = { input: 1_200, output: 80, cacheRead: 0, cacheWrite: 0, totalTokens: 1_280, cost: { input: 0, output: 0, cacheRead: 0, cacheWrite: 0, total: 0 } };

const prompt: Message = { role: "user", content: [{ type: "text", text: "Add guest checkout to the cart page." }], timestamp: 1 };
const editCall: Message = {
  role: "assistant",
  content: [{ type: "toolCall", id: "call-1", name: "edit", arguments: { path: "src/cart.ts", oldText: "a", newText: "b" } }],
  api: "anthropic-messages",
  provider: "anthropic",
  model: "claude-sonnet-4-5",
  usage,
  stopReason: "toolUse",
  timestamp: 2,
};
const editResult: Message = {
  role: "toolResult",
  toolCallId: "call-1",
  toolName: "edit",
  content: [{ type: "text", text: "Successfully replaced text in src/cart.ts." }],
  isError: false,
  timestamp: 3,
};
const steer: Message = {
  role: "custom",
  customType: "rerail.rule",
  content: "Rerail found 1 problem in src/cart.ts:\n- no-console-log: Remove console.log calls.",
  display: true,
  details: { requested: "steer", mechanism: "native-queue", sessionId: "session-1", rules: [] },
  timestamp: 4,
};
const fixCall: Message = { ...editCall, content: [{ type: "toolCall", id: "call-2", name: "edit", arguments: { path: "src/cart.ts" } }], timestamp: 5 };
const fixResult: Message = { ...editResult, toolCallId: "call-2", timestamp: 6 };
const image = { type: "image", data: "", mimeType: "image/png" };
const note: Message = {
  role: "custom",
  customType: "rerail.session-note",
  content: [image, { type: "text", text: "Rerail is watching this session." }],
  display: false,
  timestamp: 0,
};
const otherExtension: Message = { role: "custom", customType: "plan-mode.reminder", content: "Stay in plan mode.", display: false, timestamp: 7 };

const label = (text: string): string => `${RERAIL_COMPACTION_LABEL}\n${text}`;

describe("compaction guard", () => {
  it("summarizes a split turn a Rerail steer leads as history, with the person's own request", () => {
    // Pi and OMP make the steer the turn start, so the prompt lands in the history and the steer opens the prefix.
    const preparation: CompactionPreparationLike<Message> = {
      messagesToSummarize: [note, prompt, editCall, editResult],
      turnPrefixMessages: [steer, fixCall],
      isSplitTurn: true,
    };

    guardCompaction(preparation);

    expect(preparation).toEqual({
      messagesToSummarize: [
        { ...note, content: [image, { type: "text", text: label("Rerail is watching this session.") }] },
        prompt,
        editCall,
        editResult,
        { ...steer, content: label(steer.content as string) },
        fixCall,
      ],
      turnPrefixMessages: [],
      isSplitTurn: false,
    });
    expect(preparation.messagesToSummarize[1]).toBe(prompt);
  });

  it("keeps a split turn the person's prompt leads, labelling the steer inside its prefix", () => {
    const preparation: CompactionPreparationLike<Message> = {
      messagesToSummarize: [editResult],
      turnPrefixMessages: [prompt, editCall, editResult, steer, fixCall],
      isSplitTurn: true,
    };

    guardCompaction(preparation);

    expect(preparation).toEqual({
      messagesToSummarize: [editResult],
      turnPrefixMessages: [prompt, editCall, editResult, { ...steer, content: label(steer.content as string) }, fixCall],
      isSplitTurn: true,
    });
  });

  it("leaves another extension's custom message as it is, and the turn it leads split", () => {
    const preparation: CompactionPreparationLike<Message> = {
      messagesToSummarize: [prompt],
      turnPrefixMessages: [otherExtension, fixCall, fixResult],
      isSplitTurn: true,
    };

    guardCompaction(preparation);

    expect(preparation).toEqual({ messagesToSummarize: [prompt], turnPrefixMessages: [otherExtension, fixCall, fixResult], isSplitTurn: true });
    expect(preparation.turnPrefixMessages[0]).toBe(otherExtension);
  });

  it("labels copies, leaving the session's messages and the host's arrays as they were", () => {
    const steerBefore = structuredClone(steer);
    const noteBefore = structuredClone(note);
    const messagesToSummarize = [note, prompt];
    const turnPrefixMessages = [steer, fixCall];
    const recentMessages = [fixResult, steer];

    guardCompaction({ messagesToSummarize, turnPrefixMessages, isSplitTurn: true, recentMessages });

    expect([steer, note]).toEqual([steerBefore, noteBefore]);
    expect(messagesToSummarize).toEqual([note, prompt]);
    expect(turnPrefixMessages).toEqual([steer, fixCall]);
    expect(recentMessages).toEqual([fixResult, steer]);
  });

  it("labels the Rerail messages OMP keeps in recentMessages", () => {
    const preparation: CompactionPreparationLike<Message> = {
      messagesToSummarize: [prompt],
      turnPrefixMessages: [],
      isSplitTurn: false,
      recentMessages: [editCall, editResult, steer],
    };

    guardCompaction(preparation);

    expect(preparation.recentMessages).toEqual([editCall, editResult, { ...steer, content: label(steer.content as string) }]);
  });

  it("leaves the preparation as the host made it when it cannot be read", () => {
    const preparation = { messagesToSummarize: [prompt], turnPrefixMessages: null, isSplitTurn: true } as unknown as CompactionPreparationLike<Message>;

    const stderr = vi.spyOn(process.stderr, "write").mockReturnValue(true);
    try {
      expect(() => { guardCompaction(preparation); }).not.toThrow();
      expect(stderr).toHaveBeenCalledWith(expect.stringMatching(/^Rerail session_before_compact hook failed: /));
    } finally {
      stderr.mockRestore();
    }
    expect(preparation).toEqual({ messagesToSummarize: [prompt], turnPrefixMessages: null, isSplitTurn: true });
  });
});
