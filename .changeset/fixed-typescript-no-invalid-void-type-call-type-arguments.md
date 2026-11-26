---
"@neralabs/rerail-pack-typescript": patch
---

`typescript-eslint-no-invalid-void-type` no longer reports `void` as a function or method call's type argument, such as `Promise.withResolvers<void>()` or `expectTypeOf(x).toEqualTypeOf<void>()`, while generic type arguments are allowed (the default). typescript-eslint still reports these; the steer to `undefined` breaks such code.
