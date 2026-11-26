import { afterEach, describe, expect, it, vi } from "vitest";
import { send } from "../../ui/api.js";
import { failed } from "../../ui/format.js";

afterEach(() => {
  vi.unstubAllGlobals();
});

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

  it("says a change the daemon made but could not confirm on the disk in the daemon's words alone", async () => {
    const message = "the change was made, but Rerail could not confirm it is on the disk: a power cut may still undo it, and trying it again may make it twice (EIO: i/o error, fsync)";
    vi.stubGlobal("location", { origin: "http://127.0.0.1:4000" });
    vi.stubGlobal("fetch", async () => new Response(JSON.stringify({ error: "not-on-disk", message }), { status: 503 }));
    const error: unknown = await send("PUT", "/api/presentation-preferences", { presence: "silent" }).then(() => undefined, (refusal: unknown) => refusal);
    expect(failed("Couldn’t change the footer setting", error))
      .toBe("The change was made, but Rerail could not confirm it is on the disk: a power cut may still undo it, and trying it again may make it twice (EIO: i/o error, fsync).");
  });
});
