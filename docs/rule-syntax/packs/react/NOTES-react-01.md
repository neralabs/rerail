# Notes: batch react-01 (eslint-plugin-react-hooks 7.1.1)

Rules: react-hooks-error-boundaries, -exhaustive-deps, -globals, -immutability, -purity, -refs. Each was compared three
ways on both corpora the suite compares (`samples-react`, the recorded `fixtures`): its JSON original through the
product's path, its translation through `rerail-engine run-policies`, and ESLint 9.39 with eslint-plugin-react-hooks
7.1.1 itself (typescript-eslint's parser for .ts/.tsx, espree with JSX for .js/.jsx, every react-hooks rule on, inline
`eslint-disable` comments honoured), on every file. Where the original and the translation differ, ESLint decides.

## Engine gaps met by all six

- **run-policies applies no suppression comments.** Each rule declares `suppress eslint "react-hooks/…"`, but
  `run-policies` returns the diagnostics under `// eslint-disable-next-line react-hooks/x` (with or without a
  ` -- reason`, and a bare `// eslint-disable-next-line`) unmarked; the original and ESLint drop them. Probe: a
  component with `// eslint-disable-next-line react-hooks/globals` above `b += 1` is reported by the translation
  only. This is the whole of the translation-only count of exhaustive-deps (32), purity (2), globals (1) and
  immutability (1) on `samples-react`. Already noted for Bandit's `nosec` (python/NOTES-misc-untranslated.md). Engine
  change: run-policies applies (or marks) each policy's `suppress` comments as the product path does.
- **`visit_index` and `earlier` have no rows for TypeScript under run-policies** (as javascript/NOTES-misc-untranslated
  and react/NOTES-react-practice-a found). exhaustive-deps' lost-assignment check and immutability's "first change"
  and "read before its declaration" order by `node`'s start offset instead, which is the visit order for this
  grammar. Library: react-compiler.rerail's `rc_returns_non_node` (max `visit_index` of a return) and `rc_always`
  (`earlier`) silently lose their rows the same way; the same replacement fixes them.
- **A rule may redeclare a library relation unnoticed.** refs first declared `write_target(l, w)`, which
  tsx/names.rerail declares as `write_target(r, a)`; check-rr accepted the file and the two definitions merged
  (every plain `x = 1` became a "ref write"). check-rr also accepted a body atom of an undeclared relation
  (`null_check`), which run-policies then refused. check-rr should refuse both.
- **A reused tape store gave a stale result.** With one `RERAIL_TAPE_CACHE` kept across runs of edited rules, the
  fixtures corpus kept the old exhaustive-deps findings for invalid-086/087 while the same rule on those files alone
  gave the new ones; a fresh store per run (as the suite makes) gave the new ones everywhere. Not reduced further.

## Library added: tsx/react-01.rerail

`r01_live_fn(g, f)`: rc_live_fn narrowed the way the compiler (and both originals) are, used by globals and purity.
A named function becomes live only through a reference that may call it (`g()`, `h(g)`, `g.x`), not one that stores
it (`return { begin }`); a function written inside an object or array literal that is a JSX prop or a hook argument
is handed on with it (`<Palette items={[{ perform: () => … }]}/>`). Proposed for react-compiler.rerail, replacing
rc_live_fn's second clause.

## Library changes requested (existing files, not edited)

- react-compiler.rerail: `rc_returns_non_node` and `rc_always` order by `visit_index`/`earlier`; under run-policies
  they get no rows (a component returning an object is compiled anyway; an early return never makes a later call
  conditional). Order by `node`'s start offset, as the rules here do.
- react-compiler.rerail: `rc_live_fn` as `r01_live_fn` (above), and `rc_live_fn`'s users (set-state-in-render,
  static-components) rechecked with it.
- tsx: one name for "an arrow or function expression" (`fn_value` here and in ts-a-returns.rerail, `fn_expr` in
  react-class.rerail), and for `a.b or a[b]` (`mem_load` here; values.rerail's `member(n, o, x)` is the named form).
- The two models of the compiler's ref values and frozen values (refs, immutability) are each one rule's. They are
  long (refs 410 lines, immutability 275) because they port the originals' clauses; if a second rule needs ref
  values (set-state-in-effect's "the argument reads a ref"), refs' value section is the part to move to a library.

## Per rule

### react-hooks-error-boundaries: same

The compiler validates each JSX instruction in a `try` block, so an element nested in another is reported on its own
line too; the translation had reported only outermost elements. Counterexample (samples, SuperUserSagas.tsx
110-112): `try { … actionElement = (<><br /><span onClick={…}>…</span></>); } catch {…}`: ESLint and the original
report the fragment, `<br />` and `<span>`. An element inside a discarded one (`<p><i/></p>;`) is still not reported.

### react-hooks-exhaustive-deps: different (the suppression gap, and one oxlint comment)

Fixtures: same. The lost-assignment check (`useEffect(() => { value = 43; }, [])`) never fired, because it ordered
by `visit_index` (no rows): invalid-086/087 got a missing-dependency finding at the array instead of one finding per
assigned variable. Ordered by start offset now. A generator callback (`useCallback(async function* () {…})`) is an
inline function to the plugin, not "pass an inline function" (samples, excalidraw App.tsx:836).
Samples, 32 translation-only: 31 are lines under an `eslint-disable-next-line` (named or bare), the suppression gap.
The 32nd, useVoiceInput.ts:294, is under `// oxlint-disable-next-line react-hooks/exhaustive-deps`: the original
honours it, ESLint does not and reports the line, as the translation does.

### react-hooks-globals: different (the suppression gap)

Liveness now goes through tsx/react-01 (above): `<CommandPalette items={[{ perform: () => { pwaEvent = null; } }]}/>`
(samples, excalidraw App.tsx:1285) is not reported, as in ESLint and the original. The 1 translation-only line is
Maybe.test.tsx:38, under `// eslint-disable-next-line react-hooks/globals -- …`. Both sides miss useMessage.tsx:159
(`keyIndex += 1` in a function stored in the object a useMemo returns), which ESLint reports.

### react-hooks-purity: different (the suppression gap)

Liveness through tsx/react-01: `begin` in `useMemo(() => { const begin = () => … performance.now() …; return
{ begin, … }; })` is stored, not called (topPicksDnD.tsx:523/524, reported by neither ESLint nor the original).
Added the compiler's handler-in-callback case, as the original has it: `updates.map((u) => <Row onConfirm={() =>
void submit(u)}/>)` makes `submit`'s impure calls render code (ghost-updates.tsx:249/250, reported by ESLint). The 2
translation-only lines (Upload.tsx:108, Particles.tsx:51) are under `eslint-disable-next-line react-hooks/purity`.
Not followed, as before: aliases of `Date.now` (`const now = Date.now; now()`).

### react-hooks-immutability: different (the suppression gap; the original's deps-frozen clause)

Rewritten after the original's clauses, each checked against ESLint on probe files:
- A frozen value (props, a hook's argument or result, a const alias or member chain of one) changed in the
  component's own code is reported at every change; in a used function written in it (an effect, a handler, a
  useCallback body; not a function stored in a never-read const), only the first change of each name. Deeper
  functions are not checked. Before, every change anywhere was reported (slate editable.tsx: 7 lines ESLint does
  not report).
- A ref is not frozen, nor a const holding `ref.current` (`const el = capsuleRef.current; el.style.height = h`),
  unless a hook's callback read the ref above its declaration: the compiler hoists it (perseus
  use-preview-presenter.ts:180 and floating-sidebar.tsx:389, reported by ESLint).
- A `...rest` of the props is a copy (`delete props[k]` on a rest, useCol.ts:30, not reported).
- Outer values: only a module binding or an undeclared global, changed in the compiled function's own code or in a
  function written there that it calls or hands to a non-hook call.
- A parameter reassigned in the component's own code is reported, the first time, when a closure reads it
  (`bsPrefix = useBootstrapPrefix(bsPrefix)`, useCol.ts:21); pattern assignments count (`[props, ref] =
  useContextProps(…)`, Table.tsx:451).
- Memoized collections: only the methods the compiler knows for what the useMemo callback returns (array push/pop,
  Set add/clear/delete, …): `debouncedSetError.clear()` on a debounced function is not reported (Demo.tsx:139).
- Read before declaration: var, let, const and function declarations alike when a closure reads them
  (`useEffect(() => { helper(); }); function helper() {}` is reported by ESLint); in the component's own code only let
  and const; refs never.
- The useMemo-result clause: the last function of the object a useMemo callback returns that reassigns a let leads
  to the function reported at its first reassignment (use-native-format-dom.ts:58, topPicksDnD.tsx:366).
- A function that changes a local after render is reported where it is first handed on only (GoogleFields.js:153,
  not again at 162/170/178).

Left, samples: 2 original-only, useResizeObserver.test.tsx:71/73 (`elementRef.current = node` in a useCallback that
lists elementRef): the original's deps-frozen clause reports them, ESLint does not. 1 translation-only, DataFrame.tsx:445,
under `eslint-disable-next-line react-hooks/immutability`.

### react-hooks-refs: different (the suppression gap; one shorthand the original reports)

Rewritten after the original's model (98 original-only lines before). What the compiler reports is a ref's value
used, at the place it was loaded:
- Values: `r.current` (r a useRef, or a local `ref`/`…Ref`), through `!`, parentheses, `as`, ?: and && || ??; a
  member of a typed ref other than current; a member of a value; a const of render code holding one (`const prev =
  ref.current`), reported at the load while the compiler still follows it, at the read once the const is read as an
  object or after a member read of the same name; destructured consts. A finding is anchored where the original
  anchors it (`capture.LOC`): `const id = idRef.current; … <b id={id}/>` is reported at the first line.
- Uses: JSX, return, the compiled arrow's body, an argument, a condition, a test of && || ??, an operand that is used,
  a stored value, a method's receiver, a callee, an optional test, an element of a used literal (not useMemo's or
  useCallback's deps).
- Ref holders: `<div ref={splitter.ref}>` makes splitter a ref for the compiler; members of it read during render
  are reported (use-splitter.test.tsx: 22 lines, Modal.tsx, UnsplashGallery.tsx).
- Literals holding values, nested literals, and a literal passed with a value the compiler lost
  (`useState({ action: history.action })` with `history = historyRef.current`, lib.tsx:629).
- Writes (`r.current = v` outside an `if` testing that ref's current for null or falsiness; a member of a value,
  always), `+=`/`++`, null tests whose result is used, `!r.current` rendered, passed or returned.
- Functions that read refs handed to a call or `new`, tested, or used as a receiver (`new MutationObserver(onResize)`,
  `useState(() => \`${reseeds.current}\`)`); refs handed to such calls (`createObservableMap(map, mapRef, setMap)`),
  unless the call makes a ref.
- Calling a local function that reads a ref (`read()` in render) is not reported, as the original; the earlier
  translation reported it.
Left out of the original: two literals holding two different refs (`literal-lost` by ref name), `literal-passed`
(the same lines as literal-element on the corpora), `ref-beside-lost-load`, `alias-prop-load`, `alias-lost-load`.

Left, samples: 1 original-only, Modal.tsx:335, the shorthand `modalRef` (a useRef) in the object of a context value:
the original's literal-element clause reports it, and ESLint does; no value relation of the original's that I could
trace makes a bare useRef binding a ref value, so it is not modelled. Right: original. 1 translation-only,
UnitAmountInput.tsx:49, under `eslint-disable-next-line react-hooks/refs -- …`.
