# Review: react-01

Branch reviewed: `tbelanger/pack-translations-react-01` at `7efb0c9d50`. I read the six `.rerail` files, the
libraries they include (`tsx/react-compiler.rerail`, `tsx/react-01.rerail`, `tsx/react-hooks.rerail`), their JSON
originals and the plugin's source (`cjs/eslint-plugin-react-hooks.development.js`: `ExhaustiveDeps` and the
compiler's validations) before the author's NOTES and result file. The tool ran on a fleet box: ESLint 9.39.5 and
eslint-plugin-react-hooks 7.1.1 (the pack's pins), espree for `.jsx`, inline config off. Each counterexample is a
whole `.jsx` file in a directory whose `package.json` depends on `react`. Each translation ran with
`rerail-engine run-policies` on the same directory, using this branch's `docs/rule-syntax/lib`. Line:column comes
from ESLint; run-policies gives the line.

| Rule | Verdict |
|---|---|
| `react-hooks-error-boundaries` | 2 defects (useMemo callbacks, elements whose only reader is dead) |
| `react-hooks-exhaustive-deps` | 3 defects (parenthesised function, destructuring and `for…of` writes, parenthesised cleanup), 2 minor |
| `react-hooks-globals` | 2 defects (destructuring assignment, a function rendered as a JSX child) |
| `react-hooks-immutability` | 4 defects (values frozen by JSX or a hook, a function declaration handed on, a handler in an object prop, destructuring writes after render) |
| `react-hooks-purity` | 1 defect (a function rendered as a JSX child), 1 minor |
| `react-hooks-refs` | 4 defects (calling a function that reads a ref, `const { current } = ref`, a useMemo callback's result, an alias of the ref) |

## react-hooks-error-boundaries

Agreed: nested elements (one finding per element), JSX in `?:` and in an assignment to a variable read after the
try, JSX in a try with `catch (e)`, a try with JSX inside a `.map` callback or a useCallback callback (not reported by
either: the validation does not enter function expressions).

**Defect 1 (miss): a try inside a useMemo callback.** The compiler drops manual memoization before it validates, so a
useMemo callback's body becomes part of the component and its try is checked. `in_try` and `error_boundaries` need
`rc_own(e, f)` with `f` compiled, which a useMemo callback is not. `rc_render_fn` already names these callbacks.

```jsx
// b1.jsx
import { useMemo } from "react";
export function Page({ a }) {
  const el = useMemo(() => {
    try {
      return <A x={a} />;
    } catch {
      return null;
    }
  }, [a]);
  return el;
}
```

- ESLint: `b1.jsx:5:14` `react-hooks/error-boundaries`.
- translation: nothing.

**Defect 2 (extra): an element whose only reader is itself unused.** The compiler's dead-code elimination runs to a
fixed point. `unused` stops after one step: a declarator with any reference counts as used, even when that reference
is in another unused value.

```jsx
// b2.jsx
export function Page() {
  try {
    const a = <A />;
    const b = [a];
  } catch {}
  return null;
}
```

```jsx
// b6.jsx
export function Page() {
  try {
    const a = <A />;
    const b = { a };
  } catch {}
  return null;
}
```

- ESLint: nothing on either.
- translation: `b2.jsx:3`, `b6.jsx:3`.

## react-hooks-exhaustive-deps

Agreed elsewhere (my cases beyond the upstream tests): `undefined` as the dependency argument, a `satisfies` or `!`
callback (reported as not inline), a callback named in the array, `a?.b` keys, `obj.b.c = 1` keyed `obj.b`, `n++` (a
missing dependency, not a stale assignment).

**Defect 1 (extra): a parenthesised function is not taken as stable.** `isFunctionWithoutCapturedValues` finds the
function's scope by `childScopeBlock.parent === fnNode`; ESTree has no parentheses, so `const log = (() => …)` is
found. `deps_fn_site` needs the declarator's value to be the arrow itself, and tree-sitter's value is a
`parenthesized_expression`. It should take `deps_bare` of the value, as `deps_stable_site` does.

```jsx
// e1.jsx
import { useEffect } from "react";
export function Clock() {
  const log = (() => console.log("tick"));
  useEffect(() => {
    log();
  }, []);
  return null;
}
```

- ESLint: nothing.
- translation: `e1.jsx:6` ("Add `log` to this dependency array").

**Defect 2 (wrong finding): destructuring and `for…of` writes are stale assignments.** eslint-scope gives a write
reference with a `writeExpr` (the right side) to every name a pattern assigns, and to the left of a `for…of`/`for…in`
that is not a declaration. The plugin reports "Assignments to the 'x' variable … will be lost" at the right side and
stops checking the Hook. `stale` only takes an identifier that is the whole `left` of an assignment. So the
translation misses the stale assignment and reports a missing dependency instead.

```jsx
// e2.jsx
import { useEffect } from "react";
export function Pair({ items }) {
  let first;
  useEffect(() => {
    [first] =
      items;
  }, [items]);
  return first;
}
```

```jsx
// e3.jsx
import { useEffect } from "react";
export function Last({ items }) {
  let last;
  useEffect(() => {
    for (last of
      items) {}
  }, [items]);
  return last;
}
```

- ESLint: `e2.jsx:6:7` (at `items`), `e3.jsx:6:7`, both "Assignments to the '…' variable from inside React Hook
  useEffect will be lost after each render".
- translation: `e2.jsx:7`, `e3.jsx:7` ("Add `first`/`last` to this dependency array").

**Defect 3 (miss): a parenthesised cleanup.** `isInsideEffectCleanup` checks whether the function's parent is a
`ReturnStatement`, and ESTree has no parentheses. `cleanup_read` needs the `return_statement` to be the function's
direct parent, so `return (() => …)` is missed.

```jsx
// e4.jsx
import { useEffect, useRef } from "react";
export function Box() {
  const ref = useRef(null);
  useEffect(() => {
    return (() => {
      ref.current.blur();
    });
  }, []);
  return <div ref={ref} />;
}
```

- ESLint: `e4.jsx:6:11` "The ref value 'ref.current' will likely have changed …".
- translation: nothing.

**Minor (miss): a bigint const is not stable.** `isStableKnownHookValue` takes a `const` Literal whose value is a
string, a number or null. `10n` is a bigint. `deps_stable_site` accepts every tree-sitter `number`.

```jsx
// e5.jsx
import { useEffect } from "react";
export function Big({ f }) {
  const N = 10n;
  useEffect(() => {
    f(N);
  }, [f]);
  return null;
}
```

- ESLint: `e5.jsx:6:6` "React Hook useEffect has a missing dependency: 'N'".
- translation: nothing.

**Minor (wrong finding): `undefined` in the array.** In ESTree `undefined` is an Identifier, so the plugin keys it
`undefined`, an outer (unresolved) value, and reports it as unnecessary at the array. That problem then also
suppresses the construction check. tree-sitter's `undefined` node is not an `identifier`, so `deps_dkey` gives it no
key: the translation reports a complex expression at the element, and the construction check still runs.

```jsx
// e6.jsx
import { useEffect } from "react";
export function U({ f }) {
  const opts = { a: 1 };
  useEffect(() => {
    f(opts);
  }, [f, opts,
    undefined]);
  return null;
}
```

- ESLint: `e6.jsx:6:6` "React Hook useEffect has an unnecessary dependency: 'undefined'".
- translation: `e6.jsx:3` (construction of `opts`), `e6.jsx:7` (complex expression).

## react-hooks-globals

Agreed: `n++` (not reported by either), a module `let` assigned in a useMemo callback, in a function declaration
called in render, in an object method called in render, in a `forEach` callback, a handler passed as a JSX prop or
held in an array (not reported), a function passed to a custom hook (not reported), `for (n of items)` (not reported
by either).

**Defect 1 (miss): destructuring assignment.** The compiler reports each global a pattern reassigns. `globals` needs
the assignment's `left` to be an identifier.

```jsx
// g1.jsx
let a = 0, b = 0;
export function Pair() {
  [a, b] = [1, 2];
  return <b>{a}</b>;
}
```

```jsx
// g4.jsx
let last = null;
export function Track({ id }) {
  ({ last } = { last: id });
  return <b>{id}</b>;
}
```

- ESLint: `g1.jsx:3:4` and `g1.jsx:3:7`, `g4.jsx:3:6`, "Cannot reassign variables declared outside of the
  component/hook".
- translation: nothing.

**Defect 2 (miss): a function rendered as a JSX child.** `tsx/react-01.rerail` makes a named function live only
through a call, an argument or a member read (`r01_may_call`). The compiler treats a function rendered as a child as
possibly called. `react-hooks-purity`'s own header gives this case (`return <b>{get}</b>; reported`). The same gap is
purity's Defect 1 below. `r01_may_call` should also take a reference that is a `jsx_expression` child, not an
attribute's value.

```jsx
// g8.jsx
let n = 0;
export function C() {
  const bump = () => { n = n + 1; };
  return <b>{bump}</b>;
}
```

- ESLint: `g8.jsx:3:24`.
- translation: nothing.

## react-hooks-immutability

Agreed: `window.title = t` and `document.body.className = …` in render, props changed in an effect or an inline JSX
handler, a useState value changed through `const t = s`, `const { inner } = s` and `rows[0]`, an unused closure (not
reported), `x = 1` in an effect and a useCallback callback and `x += 1` in an inline handler (both places each), an
early read in an effect. Also not reported by either: `ctx.items.push(1)` on a context, `Object.assign(props, …)`,
`props = {…}` in render, `count++` in a handler, `x = 1` in a `forEach` callback inside a handler, a for-of element
of props, a `forEach` callback's parameter, `opts ?? {}`.

**Defect 1 (miss): a local value frozen by JSX or a hook, then changed.** Once a value is passed to JSX or a hook,
the compiler freezes it ("This value cannot be modified"), and a later member write is reported. `frozen` knows only
parameters, hook results and their aliases, so a component's own object or array is never frozen.

```jsx
// i1.jsx
export function List({ a }) {
  const items = [];
  const el = <Row items={items} />;
  items.push(a);
  return el;
}
```

```jsx
// i7.jsx
export function C({ a }) {
  const x = {};
  useFoo(x);
  x.a = a;
  return null;
}
```

```jsx
// i8.jsx
export function C({ a }) {
  const x = {};
  const el = <Row x={x} />;
  x.a = a;
  return el;
}
```

- ESLint: `i1.jsx:4:3`, `i7.jsx:4:3`, `i8.jsx:4:3` "This value cannot be modified".
- translation: nothing.

**Defect 2 (miss): a function declaration handed to JSX by name.** The compiler reports the reassignment and the place
the function is handed on. `handed(g, r)` follows only a `variable_declarator` holding the function, not a
`function_declaration`, so the second finding is missing.

```jsx
// i12.jsx
export function C() {
  let x = 0;
  function onClick() {
    x = 1;
  }
  return <b onClick={onClick}>{x}</b>;
}
```

- ESLint: `i12.jsx:4:5` "Cannot reassign variable after render completes", `i12.jsx:6:22` "Cannot modify local
  variables after render completes".
- translation: `i12.jsx:4` only.

**Defect 3 (miss): a handler inside an object literal prop.** `rc_deferred` counts a const function as deferred only
when every reference is directly a JSX prop's value or a hook's argument. A shorthand inside `cb={{ onClick }}` is
neither, so `late_assign` never sees the function.

```jsx
// i16.jsx
export function C() {
  let x = 0;
  const onClick = () => {
    x = 1;
  };
  return <Child cb={{ onClick }}>{x}</Child>;
}
```

- ESLint: `i16.jsx:4:5` "Cannot reassign variable after render completes".
- translation: nothing.

**Defect 4 (extra): a destructuring reassignment after render.** `assign` takes the names of an assignment pattern
(added for `[props, ref] = useContextProps(props, ref)`), so `late_assign` and the handed-on clause report them.
The compiler reports neither for a pattern in an effect or a handler.

```jsx
// i17.jsx
export function C() {
  let x = 0;
  return <b onClick={() => { [x] = [1]; }}>{x}</b>;
}
```

```jsx
// i19.jsx
import { useEffect } from "react";
export function C() {
  let x = 0;
  useEffect(() => {
    [x] = [1];
  });
  return <b>{x}</b>;
}
```

- ESLint: nothing on either.
- translation: `i17.jsx:3` (twice: the assignment and the handler), `i19.jsx:4`, `i19.jsx:5`.

## react-hooks-purity

Agreed: `new Date()` and `crypto.randomUUID()` (not reported by either), `Math.random()` in render, `useState(Date.now())`,
a default parameter, an IIFE, a `map(pick)` callback, an object method called in render, a call in a `?:` and in a
loop, `Date.now?.()`, a function passed as a JSX prop and to `map` (reported once, at the call), `useState(tick)`
with an effect calling `tick` (not reported), a useCallback result called in render (not reported).

**Defect 1 (miss): a function rendered as a JSX child.** This is the header's own example
(`const get = () => Math.random(); return <b>{get}</b>; reported`). `r01_live_fn` does not make `get` live, for
the reason given under globals' Defect 2.

```jsx
// p4.jsx
export function C() {
  const get = () => Math.random();
  return <b>{get}</b>;
}
```

- ESLint: `p4.jsx:2:21` "Cannot call impure function during render".
- translation: nothing.

**Minor (miss): the global object's own name.** The compiler resolves `globalThis.Date.now()` to the impure global.
`impure_call` needs the receiver to be the bare identifier `Date`. (`window.Math.random()` is not reported by the
tool.)

```jsx
// p6.jsx
export function C() {
  const t = globalThis.Date.now();
  const r = window.Math.random();
  return <b>{t}{r}</b>;
}
```

- ESLint: `p6.jsx:2:13`.
- translation: nothing.

## react-hooks-refs

Agreed: `ref.current.length > 0` in an `if`, `ref.current?.focus()`, an operand, a read in a `map` callback,
`useLatest`'s write, the `else` of a `!== null` test (not reported), the test of `if (!ref.current)`, `String(ref.current)`,
`ref.current += 1` and `++`, `useState(() => ref.current)`, a useCallback reading a ref handed to a call,
`setTimeout` with a writer, `console.log(ref)`, `api.track(ref)`, a `filter` callback, `?:` on a ref's value, a null
test rendered, `ref.current ?? v`, a ref's value in an array or object literal, `props.inputRef.current` (not
reported by either). Where the compiler reports one line several times (`ref.current?.focus()`: four), the
translation reports it once.

**Defect 1 (miss): calling a local function that reads a ref.** `reads_refs` is only used for functions handed to a
call, tested, or with a method called on them. A direct call `get()` in render code is not a slot for it.

```jsx
// r1.jsx
import { useRef } from "react";
export function C() {
  const ref = useRef(0);
  const get = () => ref.current;
  const v = get();
  return <b>{v}</b>;
}
```

- ESLint: `r1.jsx:5:13` (twice) "Cannot access refs during render".
- translation: nothing.

**Defect 2 (miss): `const { current } = ref`.** Destructuring `current` loads the ref's value. `alias_destructured`
needs the declarator's value to be a ref value (`rv`), and the bare ref `ref` is not one.

```jsx
// r3.jsx
import { useRef } from "react";
export function C() {
  const ref = useRef({ a: 1 });
  const { current } = ref;
  return <b>{current.a}</b>;
}
```

- ESLint: `r3.jsx:5:14`.
- translation: nothing.

**Defect 3 (miss): a useMemo callback's result.** The callback is inlined into render code (`inlined`), but its
expression body (or a `return` in it) is not a slot: `top_slot` takes only the compiled function's own returns and
an arrow that is itself compiled.

```jsx
// r7.jsx
import { useRef, useMemo } from "react";
export function C({ v }) {
  const ref = useRef(null);
  const m = useMemo(() => ref.current, []);
  const s = String(ref.current);
  return <b>{m}{s}</b>;
}
```

- ESLint: `r7.jsx:4:27`, `r7.jsx:5:20`.
- translation: `r7.jsx:5` only.

**Defect 4 (miss): an alias of the ref object.** `const local = ref` keeps the ref's type in the compiler.
`ref_obj` resolves a name through its own declaration (useRef, or a `ref`/`…Ref` name), not through a const holding
another ref.

```jsx
// r8.jsx
import { useRef } from "react";
export function C({ v }) {
  const ref = useRef(0);
  ref.current += 1;
  ref.current++;
  const local = ref;
  return <b>{local.current}</b>;
}
```

- ESLint: `r8.jsx:4:3` (twice), `r8.jsx:5:3` (twice), `r8.jsx:7:14`.
- translation: `r8.jsx:4`, `r8.jsx:5`.

## The author's result file

I checked each `different` entry's `right` side by running ESLint on the sample files twice: once honouring inline
comments, once with `noInlineConfig` (all six rules on, the pack's pins).

| Rule | Sample key | ESLint, comments honoured | ESLint, comments ignored | `right` |
|---|---|---|---|---|
| exhaustive-deps | DataFrame.tsx (streamlit, 25c859e7abbc) 255, 344, 384, 463 | nothing | each line | `original`: confirmed |
| exhaustive-deps | useVoiceInput.ts (react-spectrum, 91549941c384) 294 | 294:19, 294:32 | 294:19, 294:32 | the translation's, see below |
| globals | Maybe.test.tsx:38 | nothing | 38:3 | `original`: confirmed |
| immutability | useResizeObserver.test.tsx:71/73 (both copies) | nothing | nothing | the original is wrong there |
| immutability | DataFrame.tsx:445 | nothing | 445:5 | the translation is wrong there: `neither` confirmed |
| purity | Upload.tsx:108, Particles.tsx:51 (8daa7059ca73) | nothing | 108:23, 51:20 | `original`: confirmed |
| refs | Modal.tsx:335 (f1231418ec32) | 335:17 | 335:17 | `original`: confirmed |
| refs | UnitAmountInput.tsx:49 | nothing | 49:48 | `original`: confirmed |

exhaustive-deps' `right: original` holds for 31 of its 32 lines. The 32nd, useVoiceInput.ts:294, sits under an
`oxlint-disable-next-line` comment. ESLint does not honour it and reports the line, as the translation does. The
reason says so, but the entry's `right` field does not.

The NOTES (refs) say that calling a local function that reads a ref is not reported, "as the original". ESLint
reports it (refs Defect 1, `r1.jsx`), so the original and the translation miss it together.
