import { describe, expect, it } from "vitest";
import { ConflictHook, type ConflictHandler, type ConflictHandlerContext } from "../../../src/runtime/merge-assist/conflict-hook.js";
import type { StoredConflict } from "../../../src/runtime/merge-assist/conflict-store.js";
import { sampleConflict } from "../../support/merge-assist/status.js";

const context: ConflictHandlerContext = {
  checkout: { path: "/repo", repositoryKey: "local:repo", branch: "mine", base: "main" },
  session: null,
  settings: { hook: null },
  env: {},
  now: Date.now,
  deliver: () => undefined,
};
const stored: StoredConflict = { conflict: sampleConflict(), files: ["src/api/orders.ts"], anchor: "orders" };

describe("the conflict hook", () => {
  it("starts no handler queued behind a running one once stopped, as a daemon stopping ends its commands", async () => {
    const heard: string[] = [];
    const detected = Promise.withResolvers<void>();
    const handler: ConflictHandler = {
      name: "command",
      on: async event => {
        heard.push(event);
        if (event === "detected") await detected.promise;
      },
    };
    const hook = new ConflictHook([handler]);
    hook.fire([{ event: "detected", conflict: stored }], context, () => undefined);
    // Dismissed while its `detected` runs: `closed` waits behind it.
    hook.fire([{ event: "closed", conflict: stored }], context, () => undefined);
    await Promise.resolve();

    hook.stop();
    detected.resolve();
    await hook.settled();

    expect(heard).toEqual(["detected"]);
  });
});
