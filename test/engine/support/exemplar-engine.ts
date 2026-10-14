/**
 * A stand-in for a test-built engine, for host tests of the exemplar (exemplar-registry.ts). The native
 * `CodeRuleDslRuntime.compile` the host loads is a product build, which does not know the language; the engine's own
 * refusal of it, with this message, is checked by native tests (`code_rule_dsl::language`, where `PLANNED_LANGUAGES`
 * lists it), and its refusal of a product planned language through the real binding by
 * test/engine/code-rule/language-not-supported-engine.test.ts.
 *
 * A test file mocks the native module with it, beside the registry module (exemplar-registry.ts):
 *
 * ```ts
 * vi.mock("../../../src/engine/native/native.js", async importOriginal =>
 *   (await import("../support/exemplar-engine.js")).nativeRefusingExemplar(await importOriginal()));
 * ```
 */
import { codeRuleLanguageNotSupported } from "../../../src/engine/code-rule/code-rule-language-support.js";
import type * as NativeModule from "../../../src/engine/native/native.js";
import { EXEMPLAR } from "./exemplar-registry.js";

/** Whether a code-rule document names the exemplar as a query's or define's language. */
function namesExemplar(document: unknown): boolean {
  if (Array.isArray(document)) return document.some(namesExemplar);
  if (typeof document !== "object" || document === null) return false;
  return Object.entries(document).some(([key, child]) => (key === "language" && child === EXEMPLAR) || namesExemplar(child));
}

/**
 * `native` with a compile that refuses a rule naming the exemplar with the engine's `language-not-supported` message;
 * every other rule compiles in the real engine.
 */
export function nativeRefusingExemplar(native: typeof NativeModule): typeof NativeModule {
  let wrapped: NativeModule.NativeBinding | undefined;
  return {
    ...native,
    loadNativeBinding() {
      const binding = native.loadNativeBinding();
      wrapped ??= new Proxy(binding, {
        get(target, property) {
          if (property !== "CodeRuleDslRuntime") return Reflect.get(target, property);
          return {
            compile(source: string, format: NativeModule.CodeRuleDslSourceFormat, capabilities: string) {
              const document: unknown = JSON.parse(source);
              const unsupported = namesExemplar(document) ? codeRuleLanguageNotSupported(document) : undefined;
              if (unsupported !== undefined) throw new Error(`${unsupported.code}: ${unsupported.reason}`);
              return target.CodeRuleDslRuntime.compile(source, format, capabilities);
            },
          };
        },
      });
      return wrapped;
    },
  };
}
