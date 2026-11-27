# pt1120-typescript-01: explicit-module-boundary-types

One rule on #1120's line, checked with `translations:check` (ptcheck, engine `rerail-engine-1120`) against its JSON
original, and the differing key against typescript-eslint 8.74.0 (ESLint 9, `@typescript-eslint/parser`) on a fleet
box. Final: `same` (fixtures 74 both, rerail-typescript 13 both, no key on one side only). The rule stays per file: it
declares no `cross_file` relation, and a run with `RERAIL_SHOW_CROSSINGS=1` printed no crossing.

## typescript-eslint-explicit-module-boundary-types: same

Followed typescript-eslint (it agrees with the original here). The baseline's one original-only key,
`typescript-eslint/explicit-module-boundary-types/059.ts:2`:

```ts
const foo = arg => arg;
export = foo;
```

typescript-eslint reports 2:13 (`Argument 'arg' should be typed.`) and 2:17 (`Missing return type on function.`). The
translation reported nothing for any `export =` (`export = foo` over a const or a function declaration,
`export = (arg) => arg`). It found these through `tsx/modules.rerail`'s `export_assign`, which wants the gap before
the value to match `(?s)export\s*=\s*`. Under this engine that gap (`structural_before`) is ` = ` alone, with no
`export` in it. The same happens with `export default foo`: its gap is ` `, without `export default`. So
`export_assign` matches no `export =` at all. The rule now finds `export = v` itself: a child of the
`export_statement` whose gap matches `(?s)(export)?\s*=\s*`, so it works with or without the keyword in the gap, and
then the statement's non-comment child. That child can sit after a comment (`export = /* c */ foo`, which
typescript-eslint also reports). Checked against typescript-eslint, all agreeing after the fix: the six forms above
plus `export default 1 == 1;` (no false `export =`).

## Library change needed

`lib/tsx/modules.rerail`'s `export_assign(s)` matches no `export = v` under #1120's engine, for the reason above. Fix
it to `match("(?s)(export)?\\s*=\\s*", g)` over the gap before any child (a comment's too), and take the value as the
statement's non-comment child. Its other reader, `module_syntax`, is affected as well: it counts a file whose only
export is `export = v` as module syntax. After that fix this rule can use `export_assign` again, through a value
relation (`export_assign_value(s, v)`).

## Engine gaps

None for this rule.
