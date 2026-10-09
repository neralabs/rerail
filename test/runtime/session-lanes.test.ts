import { describe, expect, it } from "vitest";
import { SessionLanes } from "../src/session-lanes.js";

describe("SessionLanes", () => {
  it("preserves submission order within one session", async () => {
    const lanes = new SessionLanes();
    const gate = Promise.withResolvers<void>();
    const observed: string[] = [];

    const first = lanes.run("session", async () => {
      observed.push("first:start");
      await gate.promise;
      observed.push("first:end");
      return 1;
    });
    const second = lanes.run("session", async () => {
      observed.push("second");
      return 2;
    });

    await Promise.resolve();
    expect(observed).toEqual(["first:start"]);
    gate.resolve();
    await expect(Promise.all([first, second])).resolves.toEqual([1, 2]);
    expect(observed).toEqual(["first:start", "first:end", "second"]);
  });

  it("allows unrelated sessions to make progress concurrently", async () => {
    const lanes = new SessionLanes();
    const gate = Promise.withResolvers<void>();
    const observed: string[] = [];

    const blocked = lanes.run("blocked", async () => {
      observed.push("blocked:start");
      await gate.promise;
      observed.push("blocked:end");
    });
    const independent = lanes.run("independent", async () => {
      observed.push("independent");
      return "complete";
    });

    await expect(independent).resolves.toBe("complete");
    expect(observed).toEqual(["blocked:start", "independent"]);
    gate.resolve();
    await blocked;
  });

  it("continues a session after an operation rejects", async () => {
    const lanes = new SessionLanes();
    const observed: string[] = [];

    const failed = lanes.run("session", async () => {
      observed.push("failed");
      throw new Error("event failed");
    });
    const recovered = lanes.run("session", async () => {
      observed.push("recovered");
      return "ok";
    });

    await expect(failed).rejects.toThrow("event failed");
    await expect(recovered).resolves.toBe("ok");
    expect(observed).toEqual(["failed", "recovered"]);
  });
});
