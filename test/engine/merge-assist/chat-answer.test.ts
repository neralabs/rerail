import { describe, expect, it } from "vitest";
import { parseChatAnswer, type ChatConflict } from "../../../src/engine/index.js";

const RETRIES: ChatConflict = { id: "conflict:retries", shortId: "a1b2c3", title: "client.ts · withRetry" };
const SCHEMA: ChatConflict = { id: "conflict:schema", shortId: "d4e5f6", title: "schema.sql · users" };

describe("answers from chat", () => {
  it("reads each option as the whole message, case-insensitively, for the one open conflict", () => {
    expect(parseChatAnswer("Keep mine", [RETRIES])).toEqual({ kind: "answer", conflictId: RETRIES.id, option: "mine", text: null });
    expect(parseChatAnswer("  TAKE THEIRS!  ", [RETRIES])).toEqual({ kind: "answer", conflictId: RETRIES.id, option: "theirs", text: null });
    expect(parseChatAnswer("later.", [RETRIES])).toEqual({ kind: "answer", conflictId: RETRIES.id, option: "later", text: null });
    expect(parseChatAnswer("New decision: Retries live in the transport; callers pass options.", [RETRIES]))
      .toEqual({ kind: "answer", conflictId: RETRIES.id, option: "new", text: "Retries live in the transport; callers pass options." });
  });

  it("names the conflict by a leading short id or title when several are open", () => {
    expect(parseChatAnswer("#d4e5f6 take theirs", [RETRIES, SCHEMA])).toMatchObject({ kind: "answer", conflictId: SCHEMA.id, option: "theirs" });
    expect(parseChatAnswer("a1b2c3: keep mine.", [RETRIES, SCHEMA])).toMatchObject({ kind: "answer", conflictId: RETRIES.id, option: "mine" });
    expect(parseChatAnswer("schema.sql · users, keep mine", [RETRIES, SCHEMA])).toMatchObject({ kind: "answer", conflictId: SCHEMA.id });
    expect(parseChatAnswer("#a1b2c3 later", [RETRIES, SCHEMA])).toMatchObject({ kind: "answer", conflictId: RETRIES.id, option: "later" });
    expect(parseChatAnswer("#a1b2c3 new decision: take theirs' transport, but keep mine's options", [RETRIES, SCHEMA]))
      .toEqual({ kind: "answer", conflictId: RETRIES.id, option: "new", text: "take theirs' transport, but keep mine's options" });
  });

  it("reads the conflict's name after keep mine, take theirs or later too, still as the whole message", () => {
    expect(parseChatAnswer("take theirs #d4e5f6", [RETRIES, SCHEMA])).toMatchObject({ kind: "answer", conflictId: SCHEMA.id, option: "theirs" });
    expect(parseChatAnswer("keep mine (#a1b2c3)", [RETRIES, SCHEMA])).toMatchObject({ kind: "answer", conflictId: RETRIES.id, option: "mine" });
    expect(parseChatAnswer("Later #d4e5f6.", [RETRIES, SCHEMA])).toMatchObject({ kind: "answer", conflictId: SCHEMA.id, option: "later" });
    expect(parseChatAnswer("take theirs, schema.sql · users", [RETRIES, SCHEMA])).toMatchObject({ kind: "answer", conflictId: SCHEMA.id });
    expect(parseChatAnswer("take theirs into account for #d4e5f6", [RETRIES, SCHEMA])).toEqual({ kind: "none" });
    expect(parseChatAnswer("take theirs#d4e5f6", [RETRIES, SCHEMA])).toEqual({ kind: "none" });
    expect(parseChatAnswer("#a1b2c3 take theirs #d4e5f6", [RETRIES, SCHEMA])).toEqual({ kind: "none" });
  });

  it("answers the one conflict not put off when the message names none", () => {
    expect(parseChatAnswer("keep mine", [{ ...RETRIES, snoozed: true }, SCHEMA])).toMatchObject({ kind: "answer", conflictId: SCHEMA.id });
    expect(parseChatAnswer("keep mine", [{ ...RETRIES, snoozed: true }])).toMatchObject({ kind: "answer", conflictId: RETRIES.id });
  });

  it("records nothing for an ambiguous answer", () => {
    expect(parseChatAnswer("keep mine", [RETRIES, SCHEMA])).toMatchObject({ kind: "ambiguous", reason: "several conflicts are open and the message names none" });
    expect(parseChatAnswer("new decision:   ", [RETRIES])).toMatchObject({ kind: "ambiguous", reason: "a new decision without its text" });
    expect(parseChatAnswer("new decision: <text>", [RETRIES])).toMatchObject({ kind: "ambiguous", reason: "a new decision without its text" });
    expect(parseChatAnswer(`new decision: ${"x".repeat(601)}`, [RETRIES])).toMatchObject({ kind: "ambiguous" });
  });

  it("reads no answer from prose that only mentions an option", () => {
    for (const prose of [
      "take theirs into account",
      "keep mine.ts as is",
      "what does new decision: <text> mean?",
      "I made a new decision: ship Friday",
      "keep mine, or maybe take theirs",
      "don't take theirs",
      "let's look at this later, after lunch",
      "please refactor the client",
      "This note comes from Rerail, not the user.\n→ keep mine · take theirs · new decision: <text> · later",
    ]) expect(parseChatAnswer(prose, [RETRIES]), prose).toEqual({ kind: "none" });
    expect(parseChatAnswer("keep mine", [])).toEqual({ kind: "none" });
  });
});
