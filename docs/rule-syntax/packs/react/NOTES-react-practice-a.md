# React practice rules, batch react-practice-a

Six of Rerail's own React practice rules, translated by hand from `packs/react/rules/<id>.json`. There is no outside
tool: the JSON is the rule, so each translation follows it, except where the JSON engine's reading of it contradicts
the rule's own description (named below). Every rule is `same` on the suite's corpora (examples, fixtures,
samples-react). Each was also run beside its original on ad-hoc files (`useMemo` aliases, `require("react")`,
`memo(forwardRef(…))`, wrapped options, tRPC chains, type parameters, mutations, async components, `@jsxImportSource`
files, tsconfig `jsxImportSource`); the differences found there are listed per rule. The original side of an ad-hoc
run: `node --import tsx scripts/ship/product-run.ts <checkout> <corpus> <out.json> <dir holding the rule's JSON>`.

Shared vocabulary is in `lib/tsx/react-practice-a.rerail`:

- `react_api(f, x, src, form)`: callee `f` names React's `x` as a global, an import, `R.x`, `const { x } = R` or
  `const { x: y } = R`, with the import's source, so each rule keeps its own idea of React's sources (`react`, also
  `react-dom`, any case or not). Five rules use it.
- `react_object(r, src)`, `pr_global(r, x)` (the JSON's `resolves.global`: a value reference nothing in the file
  declares), `pr_import(r, src, x)`.
- `pr_fn_name(f, n)`, `pr_ident(n)`: the name a function is known by (own name, the name it is stored under, an object
  method's). fresh-dependency and hoist-static-value both name components and Hooks this way.
- `pr_react_doc(d)`: the JSON's `file.jsxRuntime: ["react", "unknown"]` gate, which all six rules use. It is
  tsx/react's `react_doc` with the package's project configuration read too (below, "Library changes needed").
- `pr_type_param(t, tp)`: a type name's type parameter (hoist-static-value).

## Review fixes (review/react-practice-a.review.md)

Each counterexample of the review is a `.test` of its rule, expecting the JSON original's lines (run as above), and
each passes.

- **All six, `@jsxImportSource` comment (review defect A).** Already fixed on the integration branch before this
  batch's review was fixed: tsx/react's `first_source_comment` orders the comments by offset since the react-04 review
  (`visit_index` has no rows under `run-policies`). The tests `review_jsx_import_source_comment` keep it fixed.
- **All six, tsconfig `jsxImportSource` (review defect B).** The engine's gate reads the package's
  `compilerOptions.jsxImportSource` (`module_graph::Config`) before its dependencies; `react_doc` read only the
  dependencies. Fixed in the batch's own library, `pr_react_doc`, not in tsx/react (which this batch does not own):
  the tsconfig.json beside the package.json (else its jsconfig.json), over a relative string `extends` chain, its own
  value first; a known library decides, an unknown package leaves it to the dependencies. Tests
  `review_tsconfig_jsx_import_source` (memo-trivial-expression has the full set: extends, jsconfig, a tsconfig without
  the option over a jsconfig with it, an unknown source, a nested tsconfig, a comment over the configuration).
- **cache-update-in-mutate-call: a receiver written over several lines.** The `queryClient`/`qc` test of a
  `refetchQueries` receiver is a whole-text match, so its `.` now crosses newlines (`(?is)`).
- **hoist-static-value: the component's type parameters.** tsx/names gives a type parameter no `ref_site`, so
  `reads_component` and `typed_by_component` never saw them. `pr_type_param` binds a type name to the `<T>` of the
  nearest generic around it that declares it (a nested generic's own `T` shadows the component's, as in the JSON's
  binding); a value or function reading it, or a `const` annotated with it, is the component's.
- **memo-trivial-expression: a multi-line async callback** is excluded (`(?s)async\b.*`), **and a dependency returned
  in parentheses** is `MEMO_IDENTITY`: `ref_same` is asked for the returned expression itself, and looks through the
  parentheses as `isSameReference` does.
- Not in the review: fresh-dependency declared its own `param_name`, which tsx/names also declares, so its clauses
  were added to names' relation (check_rr: "param_name declared 2 times"). Renamed `props_name`.

The rule cases (`packs/react/upstream/react-practice/<id>.cases.json`) are pack test data on S3; this checkout could
not pull them (no AWS credentials). The review ran them all: the only differences were the defects above.

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

Follows the JSON, with one difference: the JSON's ref and state Hook callee also accepts an identifier named `useRef`
(or `useState`, …) declared as `const useRef = React`: its site test is "a declarator whose value is React's object",
which that declarator passes. That names React's object, not its Hook, so the translation does not follow it
(react_api's shorthand and alias forms are the destructurings the JSON means). No corpus file has it.

## react-practice-fresh-dependency

Follows the JSON. A derived dependency is matched by name to a `const` of its own function's body, as the JSON's
`pattern: $X` does (no scope resolution).

## react-practice-hoist-static-value

Follows the JSON. A component read is a name whose declaration's nearest function is the top-level component or Hook
(`own`), as the JSON's `site inside stopBy fn matches top-cmp`: a function declaration's own name is no reference, so a
static function is not its own read. A type parameter's declaration is its `<T>` (`pr_type_param`).

## Library changes needed (existing files, not edited here)

- **tsx/react: `react_doc` does not read the package's `compilerOptions.jsxImportSource`.** Every React port gating
  on it treats a package that depends on React but compiles JSX with Preact (`"jsxImportSource": "preact"`) as
  React's, where the engine's gate does not. Proposed: move `pr_react_doc` and its configuration reading
  (`pr_package_runtime`, `pr_config_file`, `pr_config_source`) from lib/tsx/react-practice-a.rerail into tsx/react as
  `react_doc`, and drop `pr_react_doc` here. The reading is by lines (a string `extends` and a `jsxImportSource`
  string each on one line; an `extends` list is not followed), and finds the package folders by walking from the
  root (below, "Engine gaps").
- **tsx/names: type parameters have no `ref_site`.** `more_site` binds them for main's binding terms, but `look`
  never reaches them, so `ref_site(t, _)` has no row for a `T` read in a generic. Proposed: `ref_site(t, tp)` for a
  `type_identifier` t from the nearest enclosing `more_site(o, x, tp, "type_param")`, as `pr_type_param` does here.
- **tsx/names: a free reference.** The JSON's `resolves.global: [x]` is "a value reference to x that nothing in the
  file declares". names.rerail's `global_ref` also requires `es_global(x)`, which has no React Hook in it, so a JSON
  `resolves.global: ["useMemo"]` cannot use it. `pr_global` here is that relation; it belongs in names.rerail as
  `free_ref(r, x) :- ref(r, x, "value"), !ref_site(r, _)`.

## Engine gaps

- **Open engine request: the package's `jsxImportSource` as a repository input.** The engine computes it already
  (`module_graph::Config::jsx_import_source`, JSONC, the whole `extends` chain), but core gives a rule only
  `tsconfig(dir, base_url)` and `tsconfig_path`. A rule must read the files itself, and the engine answers
  `dir_entry`/`file_line` only for paths computed from constants ("the cone of requests reads base relation
  package_for"), so `pr_config_file` walks every folder from the root to find package folders with a configuration.
  Proposed: `.decl package_jsx_import_source(dir: symbol, source: symbol)` in core, the value `Manifest` carries
  (`DerivedPackages`), one row per package folder that has one.
- A glob's empty alternative never matches, silently: `glob("{react-use,ahooks}{,/esm,/lib}", s)` holds for
  `"react-use/esm"` and `"ahooks/lib"`, not for `"react-use"`. lifecycle-hook uses
  `match("(react-use|ahooks|…)(/(esm|es|lib|dist))?", s)`. core's text tests should accept `{,x}` or refuse it when the
  rule is read.
