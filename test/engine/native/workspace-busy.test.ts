import { describe, expect, it } from "vitest";
import { isWorkspaceBusy, loadNativeBinding } from "../../../src/engine/native/native.js";

describe("a workspace task's busy refusal", () => {
  it("is told by the message the engine exports, and nothing else", () => {
    expect(isWorkspaceBusy(new Error(loadNativeBinding().WORKSPACE_BUSY))).toBe(true);
    expect(isWorkspaceBusy(new Error("the store is busy"))).toBe(false);
    expect(isWorkspaceBusy(loadNativeBinding().WORKSPACE_BUSY)).toBe(false);
  });
});
