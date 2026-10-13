/**
 * The test-only planned language, `exemplar` (`native/test-languages/exemplar.json`): a test that needs "a planned
 * language" names it, so flipping a real language (Vue, SQL, Rust) to supported never touches a test.
 *
 * The product registry, the host module generated from it and the native build the host loads do not hold it. A host
 * test reaches it through two seams:
 * - `vi.mock` of the generated registry module with `exemplarRegistryModule()`, which lists it beside the product
 *   languages: the schema then accepts it and the host reads it as planned;
 * - `engineRefusesExemplar()` (exemplar-engine.ts), standing in for a test-built engine.
 *
 * This module imports nothing from `src/`, so the mock's factory can load it while the generated module is mocked:
 *
 * ```ts
 * vi.mock("../../../src/engine/code-rule/code-rule-language-registry.generated.js", async () =>
 *   (await import("../../support/exemplar-registry.js")).exemplarRegistryModule());
 * ```
 */
import {
  LANGUAGES_DIRECTORY,
  TEST_LANGUAGES_DIRECTORY,
  languageRegistryExports,
  readLanguageRegistry,
  type LanguageEntry,
} from "../../scripts/generate/languages/registry.js";

/** The planned language only tests see. */
export const EXEMPLAR = "exemplar";

/** The product registry and the test-only languages, in registry order. */
export function exemplarLanguageRegistry(): LanguageEntry[] {
  return readLanguageRegistry(LANGUAGES_DIRECTORY, TEST_LANGUAGES_DIRECTORY);
}

/** The exports of the generated registry module, with the exemplar among them: the factory of its `vi.mock`. */
export function exemplarRegistryModule() {
  return languageRegistryExports(exemplarLanguageRegistry());
}
