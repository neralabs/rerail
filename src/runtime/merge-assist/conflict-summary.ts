/**
 * The summary handler (docs/merge-assist-v1-build.md §12.5, the quick call): when a conflict is first detected, one
 * short call through the person's agent CLI (`AgentJudge`, counted against the same hourly budget) puts it in at most
 * 90 characters: what the other side is doing and why it collides with this branch. It runs in the background; until
 * it answers, and when it cannot (the budget is used, no judge, an error), the status line keeps the deterministic
 * fallback the detection pass gave the conflict, with why.
 */
import { createHash } from "node:crypto";
import type { ConflictSummary } from "../../engine/merge-assist/model.js";
import { stableJsonText } from "../../engine/stable-json.js";
import type { ConflictHandler } from "./conflict-hook.js";
import { fallbackSummary } from "./conflict-delivery.js";
import { newJudgeNonce, summaryJsonSchema, summaryPrompt, summaryVerdictSchema, type SummaryInput } from "./judge-prompts.js";
import { isoSeconds } from "./state.js";

/** The most quick calls one pass makes: the newest conflicts it found; the others keep their fallback line. */
export const MAX_QUICK_CALLS_PER_PASS = 3;

export const summaryHandler: ConflictHandler = {
  name: "summary",
  on: async (event, stored, context) => {
    if (event !== "detected") return;
    const { conflict, files } = stored;
    const fallback = conflict.summary ?? fallbackSummary(stored, context.now());
    if (context.detected.indexOf(conflict.id) >= MAX_QUICK_CALLS_PER_PASS) {
      return { summary: { ...fallback, via: "fallback", reason: `skipped: more than ${MAX_QUICK_CALLS_PER_PASS} conflicts were new at once` } };
    }
    const input: SummaryInput = {
      title: conflict.title,
      files,
      mine: { summary: conflict.mine.summary, hunk: conflict.mine.hunk },
      theirs: { label: conflict.theirs.label, author: conflict.theirs.author, summary: conflict.theirs.summary, hunk: conflict.theirs.hunk },
    };
    let summary: ConflictSummary;
    try {
      const outcome = await context.judge.judge({
        kind: "summary",
        repositoryKey: context.checkout.repositoryKey,
        prompt: summaryPrompt(input, newJudgeNonce()),
        jsonSchema: summaryJsonSchema,
        parse: value => summaryVerdictSchema.parse(value),
        cacheKey: createHash("sha256").update(`summary\0${stableJsonText(input) ?? ""}`).digest("hex"),
      });
      summary = outcome.ok
        ? { text: outcome.value.summary, via: "judge", at: isoSeconds(context.now()), reason: null }
        : { ...fallback, via: "fallback", reason: `${outcome.reason}: ${outcome.detail}` };
    } catch (error) {
      summary = { ...fallback, via: "fallback", reason: `failed: ${error instanceof Error ? error.message : String(error)}` };
    }
    return { summary };
  },
};
