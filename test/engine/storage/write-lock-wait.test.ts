/**
 * The lock wait's start failing, as it does from a hook bundle that left the worker's file behind: the record writer
 * that asked for it gets a rejection it handles, never a throw into the store write that met the lock.
 */
import { describe, expect, it, vi } from "vitest";
import { waitForWriteLock } from "../../../src/engine/storage/write-lock-wait.js";

vi.mock("node:worker_threads", () => ({
  Worker: class {
    constructor() {
      throw new Error("Cannot find module write-lock-wait-worker.js");
    }
  },
}));

describe("waiting for the write lock", () => {
  it("rejects, and never throws, when its worker cannot start", async () => {
    let waiting: Promise<boolean> | undefined;
    expect(() => { waiting = waitForWriteLock("/no/database.sqlite"); }).not.toThrow();
    await expect(waiting).rejects.toThrow(/write-lock-wait-worker/u);
  });
});
