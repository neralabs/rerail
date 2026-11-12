import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { formatDuration, streamProgress, type ProgressStream } from "../../src/runtime/progress.js";

/** A stream that keeps what was written, as a terminal (`isTTY`) or as a pipe. */
function recording(isTTY: boolean): ProgressStream & { text: () => string } {
  const chunks: string[] = [];
  return { isTTY, write: (text: string) => chunks.push(text), text: () => chunks.join("") };
}

/** A step that ends when the test says so. */
function pending<T>(): { run: () => Promise<T>; finish: (value: T) => void; fail: (error: Error) => void } {
  const { promise, resolve, reject } = Promise.withResolvers<T>();
  return { run: async () => await promise, finish: resolve, fail: reject };
}

describe("formatDuration", () => {
  it("reads as seconds under a minute, then minutes and seconds", () => {
    expect([0, 12_400, 59_000, 60_000, 125_000, 900_000].map(formatDuration)).toEqual(["0 s", "12 s", "59 s", "1 min", "2 min 5 s", "15 min"]);
  });
});

describe("streamProgress", () => {
  beforeEach(() => {
    vi.useFakeTimers({ now: 0 });
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("off a terminal, writes whole lines: the step, then that it is still running, then the stall warning once", async () => {
    const stream = recording(false);
    const step = pending<string>();
    const running = streamProgress(stream, { heartbeatMs: 30_000 }).step("Downloading x", step.run, { afterMs: 60_000, hint: "Check the network." });

    expect(stream.text()).toBe("Downloading x…\n");
    await vi.advanceTimersByTimeAsync(90_000);
    step.finish("done");

    await expect(running).resolves.toBe("done");
    expect(stream.text()).toBe([
      "Downloading x…",
      "Downloading x… still running after 30 s",
      // The warning is due at the same moment as the heartbeat, and comes first.
      "Downloading x has taken 1 min. Check the network.",
      "Downloading x… still running after 1 min",
      "Downloading x… still running after 1 min 30 s",
      "",
    ].join("\n"));
    // Nothing more once the step is over.
    await vi.advanceTimersByTimeAsync(120_000);
    expect(stream.text().split("\n")).toHaveLength(6);
  });

  it("on a terminal, counts the step's time up on its own line, puts the warning on a line of its own, and ends the line", async () => {
    const stream = recording(true);
    const step = pending<void>();
    const running = streamProgress(stream).step("Installing", step.run, { afterMs: 2_000, hint: "It may be stuck." });

    await vi.advanceTimersByTimeAsync(2_000);
    step.finish();
    await running;

    expect(stream.text()).toBe([
      "Installing…",
      "\r\u001B[2KInstalling… 1 s",
      "\nInstalling has taken 2 s. It may be stuck.\nInstalling… 2 s",
      "\r\u001B[2KInstalling… 2 s",
      "\n",
    ].join(""));
  });

  it("rejects as the step does, and stops reporting it", async () => {
    const stream = recording(true);
    const step = pending<void>();
    const running = streamProgress(stream).step("Checking", step.run);
    step.fail(new Error("npm failed"));

    await expect(running).rejects.toThrow("npm failed");
    expect(stream.text()).toBe("Checking…\n");
    await vi.advanceTimersByTimeAsync(5_000);
    expect(stream.text()).toBe("Checking…\n");
  });
});
