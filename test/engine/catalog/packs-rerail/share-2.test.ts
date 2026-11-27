/**
 * The pack validation contract for share 2 of the packs with rules written in `.rerail`
 * (test/engine/support/pack-suite.ts, `rerailPackShare`); test/engine/catalog/packs.test.ts validates every other pack.
 */
import { vi } from "vitest";
import { describePacks, rerailPackShare } from "../../support/pack-suite.js";

// As in packs.test.ts, so every pack is validated against the same registry and engine.
vi.mock("../../../../src/engine/code-rule/code-rule-language-registry.generated.js", async () =>
  (await import("../../support/exemplar-registry.js")).exemplarRegistryModule());
vi.mock("../../../../src/engine/native/native.js", async importOriginal =>
  (await import("../../support/exemplar-engine.js")).nativeRefusingExemplar(await importOriginal()));

describePacks(rerailPackShare(2));
