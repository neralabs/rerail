/**
 * A stand-in for a test-built engine, for host tests of the exemplar (exemplar-registry.ts). The native
 * `CodeRuleDslRuntime.compile` the host loads is a product build, which does not know the language; the engine's own
 * refusal of it, with this message, is checked by native tests (`code_rule_dsl::language`, where `PLANNED_LANGUAGES`
 * lists it).
 *
 * A test file mocks the native module with it, beside the registry module (exemplar-registry.ts):
 *
 * ```ts
 * vi.mock("../../../src/engine/native/native.js", async importOriginal =>
 *   (await import("../../support/exemplar-engine.js")).nativeRefusingExemplar(await importOriginal()));
 * ```
 */
import { codeRuleLanguageNotSupported } from "../../src/engine/code-rule/code-rule-language-support.js";
import type * as NativeModule from "../../src/engine/native/native.js";

/**
 * `native` with a compile that refuses a rule of the exemplar, or one using a capability its language has
 * not-supported-yet, with the engine's `language-not-supported` message; everything else compiles in the real engine.
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
              const unsupported = codeRuleLanguageNotSupported(JSON.parse(source));
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
