# React rules, batch react-02

Six rules of eslint-plugin-react 7.37.5 and eslint-plugin-react-hooks 7.1.1, checked against their JSON originals
(`npm run translations:check -- --pack react`) and, for every key one side reports alone, against the tool itself:
ESLint 9.39 with both plugins and the typescript-eslint parser, run on the corpus file, once with inline directives
honoured and once with `noInlineConfig` (ESLint's view without the file's `eslint-disable` comments).

New library: `lib/tsx/react-02.rerail` (set-state-in-effect and set-state-in-render): `rcx_before` (start-offset
order), `rcx_setter_site` / `rcx_set_state` (a useState setter, `[, setV]` included). set-state-in-effect also
includes `lib/tsx/react-practice-a.rerail` for `react_api` (is the effect hook React's).

## Engine gaps found

1. **`run-policies` does not apply a `.violation`'s `suppress` lines.** A probe file
   `<div>{/* eslint-disable-next-line react/jsx-no-comment-textnodes */}\n  // x\n</div>` is still reported by
   react-jsx-no-comment-textnodes's translation, and so is one under a file-level
   `/* eslint-disable react/jsx-no-comment-textnodes */`. The originals honour these comments (the product's
   findings layer), and so does ESLint. Every remaining difference of jsx-key and jsx-no-comment-textnodes, and most
   of rules-of-hooks' and set-state-in-effect's, is such a comment: ESLint with directives honoured reports none of
   those keys, with `noInlineConfig` it reports each of them. The rules declare the right `suppress` lines; nothing
   in a rule can fix this short of re-implementing ESLint's directives in it.
2. **`visit_index` and `earlier` have no rows for TypeScript** under `run-policies` (as NOTES-react-practice-a and
   NOTES-misc-untranslated found). A probe `rc_set_state(c), visit_index(c, j)` holds for no call. Two library
   relations read them, so they are wrong today: tsx/react-compiler's `rc_always` (its "no return before n" never
   excludes anything: `if (x) return null; setN(1);` counts as unconditional) and `rc_returns_non_node` (the "last
   return" max is over an empty set, so a block-bodied function returning an object still counts as a component).
   This batch reads order off `node`'s start offsets instead (`rcx_before`).
3. **The tape keeps no comma tokens in an array pattern.** `prev_token(s, ",")` is refused by check-rr ("prev_token
   reads `,`, which the tape leaves out in arguments"), and `token(p, …)` of `[b, c]` lists only `b` and `c`.
   `rcx_setter_site` reads a hole off the pattern's text instead.

## Library changes proposed (existing files, not edited here)

- tsx/react-compiler: `rc_always` and `rc_returns_non_node` order by start offset (`node(_, n, _, s, _)`), not
  `earlier`/`visit_index`. Then set-state-in-render's local `always` can go.
- tsx/react-compiler: `rc_state_setter` should accept `const [, setV] = useState(…)` (the compiler types the second
  element whatever precedes it); `rcx_setter_site` is that, and could replace it.

## react-hooks-set-state-in-effect: different, the original right on 7 keys (suppressions), the translation on 1

Revision 2 reported nothing: `visit_index` (the "first call" of each effect) and `earlier` (setter declared before
its call) had no rows. Fixed, and then made to follow the compiler's `validateNoSetStateInEffects` /
`getSetStateCall` (read in the plugin's bundled compiler):

- A function a component stores is followed whether written in place or wrapped in `useCallback` (the compiler
  drops manual memoization before validating): `const hide = useCallback(() => setOpen(false), []);
  useEffect(() => { if (x) hide(); })` is reported at `hide()`, and `useLayoutEffect(update, [])` at `update`.
- `const [, force] = useState(0)` is a setter (`rcx_set_state`).
- Ref-derived values (enableAllowSetStateFromRefsInEffects, on by default): a value is derived when it reads a ref
  (`useRef`'s result, a name `ref`/`…Ref`) or a variable of the function assigned a derived value (declarations,
  destructuring, assignments, `o.k = el`); any expression containing one is derived. The compiler walks the
  function in order and takes the first setter call that a derived test does not control; when that call's first
  argument is derived it stops with no finding. Counterexamples, each checked with ESLint:
  - `const i = ref.current; setN((p) => p + i);` not reported (the argument captures a derived value)
  - `const s = compute({ el: ref.current }); setN(s.h);` not reported
  - `const el = ref.current; if (el && el.a < el.b) setN(1); else setN(2); setN(3);` reported at `setN(3)` only;
    the original misses it
  - `if (!ref.current) return; setN(3);` not reported; `if (!ref.current) return; switch (s) { case 1: setN(2); }`
    reported (the switch's test decides that block, not the early return); `if (!ref.current) throw …; setN(5)`
    reported (a throw is no exit for the compiler's post-dominators)
  - `let a = v; if (keyRef.current) { a = 1; } setN(a);` not reported: `a` is a phi with an operand from a block a
    ref controls (superset RangeFilterPlugin.tsx's `handleChange`, so its caller's effect at :618 is not reported)
- The effect hook must be React's: `import useLayoutEffect from 'use-isomorphic-layout-effect'` is not (react-select
  Menu.tsx:344, :631 and their copies: neither ESLint nor the original reports them).
- A local function that names itself (`const check = useCallback((n) => { … setTimeout(() => check(n + 1)) … })`)
  is not followed: the compiler holds it as a context variable, which this validation does not track (material-ui
  Ad.tsx:188). ESLint on `const f = (k) => { setN(k); if (k) setTimeout(() => f(k - 1)); }; useEffect(() => {
  f(1); })` reports nothing; with `function f(k) { setN(k); }` it reports `f(1)`.

The last run (ptcheck below) leaves 8 translation-only keys on samples-react, 0 original-only, examples and fixtures
the same:

- 7 are under `eslint-disable` comments (gap 1): AppFrameBanner.tsx:29, render-error.tsx:73,
  BenefitSearchComplex.tsx:100 and :111, useComboBox.ts:430, SidebarNav.tsx:211 and :237. Right: original.
- polar BenefitGrant.tsx:134, `let remaining = seconds; setRetryCountdown(remaining);` after two early returns
  whose tests (`!retryAfter`, `isNaN(seconds) || seconds <= 0`) read no ref: ESLint reports it, the original does
  not. Right: translation.

## react-hooks-set-state-in-render: same

Two fixes. An async useMemo callback is reported once, at the callback, when a setter in it is unconditional (the
JSON's `async-memo` clause; ESLint: `useMemo(async () => { setN(1); await x; setN(2); })` gives one finding, at the
callback's line); revision 2 reported each setter instead (AuthProvider.tsx:162 and :170 against the original's
:158). And `always` orders returns by start offset (gap 2): `if (x) return null; setN(1);` is not reported, as in
ESLint and the original. A sync useMemo callback's setters are each reported, conditional or not, as ESLint does
(`useMemo(() => { if (x) setN(1); return 1; })` is reported at the setter).

## react-hooks-static-components: same

Revision 2 reported a tag anywhere inside the component; the compiler's `validateStaticComponents` reads only the
component's own HIR, where a useMemo callback is inlined and every other nested function is not. Checked with
ESLint:

- `const C = () => <div/>; return <div>{items.map(() => <C />)}</div>;` not reported (tag in a callback):
  blueprint iconsNext.tsx:161, the react-admin stories, tinacms rich-text, and the plugin's own
  no-unstable-nested-components invalid-018/024 fixtures.
- `const body = useMemo(() => <div><H /></div>, [x]);` with `H` made in render: reported (superset
  ExploreChartPanel.tsx:377).
- `const H = registry.get(k) ?? Default;` reported: a phi takes either side of `??`/`&&`/`||`, so the left
  operand counts too (ghost app-icon.tsx:41, :77; superset ResultSet.tsx:771); `const G = Named ?? X` where `Named`
  is made in render: a name holding a made value counts.
- A nested `function Row() {}` declaration is made in render (invalid-000).

## react-hooks-rules-of-hooks: different, neither side right everywhere

28 keys only the translation reports, all on samples-react:

- 23 are under a suppression comment the product honours and `run-policies` does not (gap 1): 19 `eslint-disable`
  (react-router lib.tsx, Theme.tsx, GridLayout.ts, AppFrameBanner.tsx, …) and 4 under
  `// oxlint-disable react-hooks/rules-of-hooks` (radix avatar.tsx:198, 200, 201, 216). Right: original.
- 5 ESLint reports and the original misses. Right: translation.
  - `import { use } from 'chai'; use(chaiAsPromised)` at the top level (cypress parse-changelog-spec.ts:6): the
    plugin knows a Hook by its name, not its import.
  - `Md.use(MdTaskList)` at the top level (lepton markdown/index.js:13): a capitalised namespace's `use`.
  - Playwright fixtures `app: async ({ task }, use) => { … await use(app) … }` (polar e2e/utils/test.ts:48, 54,
    57): `use` in an async function.

## react-jsx-key: different, the original right (the translation's suppress lines are not applied)

36 translation-only keys, all under `eslint-disable` comments (gap 1): `/* eslint-disable react/jsx-key */` at the
top of the slate benchmarks and koenig SettingsPanel.stories.jsx, `// eslint-disable-next-line react/jsx-key` in
gatsby plugins.tsx:74, :77, ant-design useModal/index.tsx:89, superset controlPanel.tsx:167, :221. ESLint reports all
36 with `noInlineConfig` and none with directives honoured.

## react-jsx-no-comment-textnodes: different, the original right (the translation's suppress lines are not applied)

One translation-only key, polar Monetization.tsx:69, under a file-level
`/* eslint-disable react/jsx-no-comment-textnodes */` (gap 1). ESLint reports it only with `noInlineConfig`.

## Check

`ptcheck --files <the four changed files> -- --pack react --rules <the six>` on integration commit 67a5689e1a:
2 same (set-state-in-render, static-components), 4 different, 0 failed; every corpus compared; exit 0. Counts in
`react-02.result.json`. `equivalence.json` still lists set-state-in-render and static-components as different and
the old counts of the other four; the coordinator updates it.
