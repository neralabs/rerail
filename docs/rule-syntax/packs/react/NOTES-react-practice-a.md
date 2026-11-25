# React practice rules, batch react-practice-a

Six of Rerail's own React practice rules, translated by hand from `packs/react/rules/<id>.json`. There is no outside
tool: the JSON is the rule, so each translation follows it, except where the JSON engine's reading of it contradicts
the rule's own description (named below). Every rule is `same` on the suite's corpora (examples, fixtures,
samples-react). Each was also run beside its original on ad-hoc files (`useMemo` aliases, `require("react")`,
`memo(forwardRef(…))`, wrapped options, tRPC chains, type parameters, mutations, async components, `@jsxImportSource`
files); the differences found there are listed per rule.

Shared vocabulary is in `lib/tsx/react-practice-a.rerail`:

- `react_api(f, x, src, form)`: callee `f` names React's `x` as a global, an import, `R.x`, `const { x } = R` or
  `const { x: y } = R`, with the import's source, so each rule keeps its own idea of React's sources (`react`, also
  `react-dom`, any case or not). Five rules use it.
- `react_object(r, src)`, `pr_global(r, x)` (the JSON's `resolves.global`: a value reference nothing in the file
  declares), `pr_import(r, src, x)`.
- `pr_fn_name(f, n)`, `pr_ident(n)`: the name a function is known by (own name, the name it is stored under, an object
  method's). fresh-dependency and hoist-static-value both name components and Hooks this way.

## react-practice-memo-trivial-expression

Follows the JSON. The dependency comparison (`compare … text: "reference"`) is ESLint's `isSameReference` with static
keys: tsx/eslint-b-control's `ref_same(a, b, "static")`.

## react-practice-callback-for-dom-only

Follows the JSON, including what it leaves out: `const { useCallback: cb } = React` is not React's useCallback there
(only the shorthand is), so `form != "alias"`.

## react-practice-cache-update-in-mutate-call

Follows the rule's description, not the JSON engine, for options stored in a variable. The JSON finds the variable
through `inside program, has identifier $OPTS that is a mutate call's second argument`; the JSON engine compares
`$OPTS` only against the first such identifier in the file and does not try the later ones, so only the first
variable passed to any `mutate()` counts:

```tsx
const o1 = { onSuccess: () => queryClient.invalidateQueries() };
const o2 = { onSuccess: () => queryClient.removeQueries() };
m.mutate(v, o1);
m.mutate(v, o2);      // original: o1's callback only; translation: both
```

The same holds for a `useMemo` factory stored in a variable (`const opts = useMemo(() => ({ onSuccess … }), [])`):
the original reports it only when `opts` is the first name passed as a mutate call's second argument in the file. The
translation reports every variable some mutate call passes (`passed_opts`). Right: translation. No corpus file shows
it.

## react-practice-lifecycle-hook

Follows the JSON, with two differences:

- The JSON's ref and state Hook callee also accepts an identifier named `useRef` (or `useState`, …) declared as
  `const useRef = React`: its site test is "a declarator whose value is React's object", which that declarator passes.
  That names React's object, not its Hook, so the translation does not follow it (react_api's shorthand and alias
  forms are the destructurings the JSON means). No corpus file has it.
- Files whose JSX runtime a `@jsxImportSource` comment names as another library (`/** @jsxImportSource preact */`)
  are skipped by the original and checked by the translation. That is a gap of tsx/react (below), shared by every
  React translation that gates on `react_doc`. Right: original.

## react-practice-fresh-dependency

Follows the JSON. A derived dependency is matched by name to a `const` of its own function's body, as the JSON's
`pattern: $X` does (no scope resolution).

## react-practice-hoist-static-value

Follows the JSON. A component read is a name whose declaration's nearest function is the top-level component or Hook
(`own`), as the JSON's `site inside stopBy fn matches top-cmp`: a function declaration's own name is no reference, so a
static function is not its own read.

## Library changes needed (existing files, not edited here)

- **tsx/react: `@jsxImportSource` and `@jsx` comments are never read.** `first_source_comment` and `pragma_comment`
  take the first comment by `visit_index`, and `visit_index` has no rows under `rerail-engine run-policies` (a probe
  rule: `kind(c, "comment"), visit_index(c, i)` holds for no comment, while `kind(c, "comment"), comment(c, t),
  contains("@jsxImportSource", t)` holds). So `react_doc` never excludes a file by its comment. Proposed: order the
  comments by start offset instead, `i = min s : { kind(n, "comment"), docof(n, d), comment(n, t),
  contains("@jsxImportSource", t), node(d, n, _, s, _) }`, then `node(d, c, "comment", i, _)`; the same for
  `pragma_comment`.
- **tsx/names: a free reference.** The JSON's `resolves.global: [x]` is "a value reference to x that nothing in the
  file declares". names.rerail's `global_ref` also requires `es_global(x)`, which has no React Hook in it, so a JSON
  `resolves.global: ["useMemo"]` cannot use it. `pr_global` here is that relation; it belongs in names.rerail as
  `free_ref(r, x) :- ref(r, x, "value"), !ref_site(r, _)`.

## Engine gaps

- A glob's empty alternative never matches, silently: `glob("{react-use,ahooks}{,/esm,/lib}", s)` holds for
  `"react-use/esm"` and `"ahooks/lib"`, not for `"react-use"`. lifecycle-hook uses
  `match("(react-use|ahooks|…)(/(esm|es|lib|dist))?", s)`. core's text tests should accept `{,x}` or refuse it when the
  rule is read.
