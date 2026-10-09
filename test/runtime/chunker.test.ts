import { describe, expect, it } from "vitest";
import { StreamingSampleChunker } from "../../src/runtime/chunker.js";

describe("StreamingSampleChunker", () => {
  it("emits complete sentences across streaming deltas", () => {
    const chunker = new StreamingSampleChunker({ maxCharacters: 1_000 });

    expect(chunker.append("message-1", "Guest checkout remains avail")).toEqual([]);
    expect(chunker.append("message-1", "able. Returning users keep")).toEqual([
      "Guest checkout remains available.",
    ]);
    expect(chunker.append("message-1", " their address\nAnalytics follow.")).toEqual([
      "Returning users keep their address",
    ]);
    expect(chunker.finish("message-1")).toEqual({
      remainder: "Analytics follow.",
      message: "Guest checkout remains available. Returning users keep their address\nAnalytics follow.",
    });
  });

  it("isolates streams and drops aborted state", () => {
    const chunker = new StreamingSampleChunker({ maxCharacters: 1_000 });
    chunker.append("one", "First stream");
    chunker.append("two", "Second stream.");
    chunker.abort("one");

    expect(chunker.finish("one")).toBeNull();
    expect(chunker.finish("two")).toEqual({
      remainder: "Second stream.",
      message: "Second stream.",
    });
  });

  it("bounds unpunctuated streams to recent text", () => {
    const chunker = new StreamingSampleChunker({ maxCharacters: 12 });
    chunker.append("long", "0123456789abcdefghijkl");

    expect(chunker.finish("long")).toEqual({
      remainder: "abcdefghijkl",
      message: "abcdefghijkl",
    });
  });
});
