import { resolve } from "node:path";
import { describe, expect, it } from "vitest";
import { loadNativeBinding } from "../src/native.js";

describe("native model", () => {
  it("loads the pinned local artifact and exposes its fingerprint", () => {
    const native = loadNativeBinding();
    const engine = native.ConversationEngine.load(resolve("model"), 256);

    expect(engine.fingerprint()).toEqual({
      modelId: "minishlab/potion-base-8M",
      revision: "bf8b056651a2c21b8d2565580b8569da283cab23",
      dimensions: 256,
      weightsSha256: "a4264cfe4354253b9332731aa3db0c066f302c8507ebd791ba1b10f64efc8eb5",
      tokenizerSha256: "273ca9e28ec6990aea6206b0364443754d87e87a5dd28e94026ea9999ba3bf62",
      kernelFormatVersion: 1,
    });
  });
});
