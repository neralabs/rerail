import { describe, expect, it } from "vitest";
import { failed } from "../../ui/format.js";

describe("a failure toast", () => {
  it.each([
    ["the daemon did not answer within 30 s", "Couldn’t save. The daemon did not answer within 30 s."],
    ["PUT /api/repository/settings answered 503", "Couldn’t save. PUT /api/repository/settings answered 503."],
    ['PUT /api/x answered 403: {"error":"nope"}', 'Couldn’t save. PUT /api/x answered 403: {"error":"nope"}'],
    ["Already a sentence.", "Couldn’t save. Already a sentence."],
    ["  spread\nover lines  ", "Couldn’t save. Spread over lines."],
  ])("says what failed, then %j as a sentence", (message, text) => {
    expect(failed("Couldn’t save", new Error(message))).toBe(text);
  });

  it("says only what failed when the error has no message", () => {
    expect(failed("Couldn’t save", new Error(""))).toBe("Couldn’t save.");
  });
});
