---
"@neralabs/rerail-pack-typescript": patch
---

TypeScript pack: `typescript-eslint-no-unused-expressions` no longer reports `as`/`satisfies` type tests whose type is a name with two or more dots (`z.email() satisfies z.core.$ZodStringFormatTypes;`), matching typescript-eslint. `x as A.B.C;` around a non-call is still reported, as upstream does.
