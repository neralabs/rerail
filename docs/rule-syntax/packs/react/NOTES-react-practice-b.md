# react practice rules, batch b

Five React practice rules, with no upstream linter: the JSON original is the specification, so each translation
follows it, corner cases included. All five are `same` on examples, fixtures and samples-react. Ad hoc cases (about 70
findings over the five rules: aliases, wrappers, guards, gates) were run through both sides on a fleet box and gave
the same keys.

## Library: `lib/tsx/react-practice-b.rerail` (new)

Four of the rules ask which callee is React's Hook, and two which name is a useState setter. The library gives
`rp_hook(f, x, h)` (an undeclared `x`, `import { x as f } from "react"`, `React.x`, `const { x } = React`; `h` says
whether it goes through `require("react")`, which only request-beside-data-library accepts), `rp_hook_call`,
`rp_react_object`, `rp_undeclared` (main's `resolves.global: [x]`: no declaration in the file binds the name),
`rp_destructured`, `rp_inner` (a wrapper's first child, comments skipped) and `rp_state_setter`.

`rp_state_setter` follows the JSON's `follows: ","` exactly: the element right after the pattern's first comma, with
no comment between. The tape leaves separator commas out, so `prev_token(p, ",")` is refused; a separator comma is
read from `structural_before` and a hole's comma from `slot_comma`. Counterexamples, both sides agree:
`const [a, /* c */ setA] = useState()` has no setter; `const [, setA]` and `const [a, b, c]` (b) have one.

No existing library is changed. No engine change is needed.

## Per rule

- **mounted-ref-never-reset**: follows the JSON. The ref's binding value must be `useRef(true)` itself (one argument,
  `true`); `useRef(true as boolean)` is not one, and `(useRef(true))` is.
- **request-beside-data-library**: follows the JSON, its six clauses as one rule with three ways the result reaches
  a setter and two gates. The "stored, then passed later" await form reads `inside … precedes` as: an ancestor of the
  request, below the function's body, with a later sibling that passes an awaited name to a setter. A file that does
  not parse is the original's `source-incomplete` failure and is withheld from the translation: neither reports it.
- **selector-returns-new-collection**: follows the JSON, which counts only `"react"` (not react-dom, case-sensitive)
  as React's module here, unlike the other four; `const { x: useCallback } = React` counts as useCallback, as the
  JSON's name test does.
- **side-effect-in-render**: follows the JSON. Its router-Hook wrappers use `nthChild: 1` without `ofRule`, so a comment
  is the first child: `(/* c */ useRouter()).push("/x")` is not reported by either side.
- **then-without-stale-guard**: follows the JSON. A cleanup named by a function declaration
  (`function stop() { alive = false } … return stop`) is not followed: a declaration has no binding value on either
  side, so that effect is reported. A dependency is matched to the request's argument (and to a keyed write's key)
  as the JSON's `pattern: "$D"` matches trees: by text with the white space between tokens left out (`tws_flat`),
  one space kept between two word characters, and, where two texts differ only in white space, by their trees'
  kinds, child counts and leaf texts (`tws_differ`, so white space inside a string or a comment still counts). So
  `session.user\n  .id` and `session . user . id` are `session.user.id`; `session.user/* c */.id`,
  `(session.user).id`, `session?.user.id`, `a['x'].id` against `a["x"].id`, and `a["x  y"].id` against
  `a["x y"].id` are not, on both sides. Until the review, the text was compared as written, so a dependency split
  across lines was not found (review react-practice-b; its two cases are the rule's first two `.test`s, the third
  holds the cases above).

## Engine gaps

None found for these rules.
