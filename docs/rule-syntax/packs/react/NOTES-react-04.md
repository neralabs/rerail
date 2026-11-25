# react-04: six eslint-plugin-react rules against their originals

Rules: `react-no-find-dom-node`, `react-no-render-return-value`, `react-no-string-refs`,
`react-no-unescaped-entities`, `react-no-unknown-property`, `react-no-unsafe`.

Ground truth: ESLint 9 with eslint-plugin-react 7.37.5, `@typescript-eslint/parser` with JSX on for every file,
`settings.react.version` "19.0.0" (the recording assumes the latest React), each rule at "error", run over the
`samples-react` corpus and the rules' fixtures on a fleet box. The pack's own configuration
(`packs/react/upstream/common.mjs`) has no settings, which means the latest React; for these six rules the two
answer the same (they test the version only against 0.13 to 19).

## Results

| Rule | Status | Difference |
|---|---|---|
| react-no-find-dom-node | different | samples-react: 11 translation-only, all suppressed by an eslint-disable comment |
| react-no-render-return-value | different | samples-react: 3 translation-only, same cause |
| react-no-string-refs | different | samples-react: 5 translation-only, same cause |
| react-no-unescaped-entities | different | samples-react: 20 translation-only, same cause |
| react-no-unknown-property | different | samples-react: 24 translation-only, same cause |
| react-no-unsafe | different | fixed (reported nothing); now samples-react: 8 translation-only, same cause |

## The one remaining difference: run-policies does not apply `suppress`

Every translation-only key in this batch is a finding that an `eslint-disable` comment for the rule silences:

    // eslint-disable-next-line react/no-string-refs
    ref="input"                                        perseus …/expression.tsx:245
    /* eslint-disable react/no-unknown-property */     material-ui …/integerPropType.test.js (15 keys)
    anchorEl: findDOMNode(this.button), // eslint-disable-line react/no-find-dom-node    react-admin …/FilterButton.js:39

ESLint and the original honour these comments. Each translation declares the same `suppress eslint "react/…"` lines as
the JSON, but `rerail-engine run-policies` (`tools::run_policies`) returns its diagnostics with no suppression applied
or marked, so the suite counts them. Proof that this is the only cause: with `linterOptions.noInlineConfig: true`
(ESLint ignores every disable comment), ESLint's (path, line) keys on samples-react equal the translations' keys
exactly, for all six rules (0 extra on either side); with inline config on, ESLint's keys equal the original's
(`both` counts 154, 19, 85, 132, 290, 21). The python batch found the same gap for `# nosec`
(`python/NOTES-misc-untranslated.md`). Engine gap: run-policies should apply the policy's `suppress` comments as
the product path does. Not fixed in the rules: an in-rule model of eslint-disable would duplicate the engine's
suppression and be wrong once run-policies applies it.

## react-no-unsafe: visit_index has no rows

The rule picked the plugin's first member with a name (`Array.prototype.find` in `checkUnsafe`) as
`min i : { member_named(k, x, m2), visit_index(m2, i) }`. In `run-policies` on TypeScript, `visit_index` (and
`earlier`) yield no rows (`v(m, i) :- kind(m, "method_definition"), visit_index(m, i).` reports nothing on the
fixtures), so the rule reported nothing anywhere: 1 + 12 + 21 original-only keys. The members of a class body or a
createReactClass spec are siblings, so "first" is now "no earlier sibling has the name", by `precedes`. Checked
against ESLint on ad-hoc cases, all matching by line:

    class A extends React.Component { UNSAFE_componentWillMount() {} … UNSAFE_componentWillMount() {} }   line of the first
    class B extends React.Component { @dec @dec2 UNSAFE_componentWillUpdate() {} }                    at the first decorator
    createReactClass({ UNSAFE_componentWillMount: function () {}, UNSAFE_componentWillMount() {} })     the first
    class D { UNSAFE_componentWillMount() {} }                                                         not a component: none

Fixtures `valid-007`/`valid-009` are valid upstream only under `settings.react.version` < 16.3; at the latest React
ESLint reports them, as do the original and the translation.

Engine gap: `visit_index`/`earlier` (core.rerail `.input`) produce no rows for TypeScript under run-policies. Any rule
that orders by them is silently empty there.

## Review (react-04): the defects found, and their fixes

An independent review (`review/react-04.review.md` on `tbelanger/pack-translations-react-04-review`) found 16 cases
where a translation and ESLint disagree. All 16 were right; each is fixed and is a `.test` of its rule (the review's
counterexamples, each test checked to fail on the earlier library). Checked against the pinned ESLint on the review's
44 case files and 10 more of mine (`run-policies` against `eslint --no-inline-config`, by rule, path and line): no
difference left on either side.

- **react-no-find-dom-node** matches ESTree's CallExpression: the callee through parentheses and not a tagged
  template (`called`, tsx/eslint-a-common), a private `this.#findDOMNode()` by ESTree's name (`estree_name`), and a
  computed `api[findDOMNode](…)` whose property is the identifier. Before: `(findDOMNode)(c)`, `this.#findDOMNode()`
  and `api[findDOMNode](this)` missed, ``findDOMNode`…` `` reported.
- **react-no-render-return-value**: the same callee (`(ReactDOM.render)(…)`, `(ReactDOM).render(…)` and
  `ReactDOM[render](…)`, not a tagged template), and a computed key keeps the value: its ESTree parent is the
  Property (a pair, a pattern's pair, or an object literal's method; not a class member). Before:
  `{ [ReactDOM.render(…)]: true }` and `ReactDOM[render](…)` missed, ``ReactDOM.render`…` `` reported.
- **react-no-string-refs** reads the value with `prop_value`, which skips comments in the braces. Before:
  `ref={/* legacy */ "a"}` missed.
- **react-no-unknown-property**: `<this …>`'s name has the kind `this` in the JavaScript grammar. Before: missed in
  `.jsx`.
- **react-no-unsafe**: all seven were in the shared library, so every react rule that reads class components gets
  the fixes (below). Before: JSDoc-declared components, `new createReactClass({…})`, `extends (React.Component)`,
  `abstract UNSAFE_…(): void`, `[(UNSAFE_…)]() {}` missed; with overloads the implementation reported instead of the
  first signature; `/** @jsx h */` not read, so `React.Component` stayed a component and `h.Component` was none.

## Library changes

- `tsx/react`'s `pragma_comment` and `first_source_comment` took the first comment by `visit_index`, which has no
  rows under run-policies (above), so neither `@jsx` nor `@jsxImportSource` was ever read; they take the comment
  that starts first (`node`'s offset), and read a multi-line comment (`(?s)`). `pragma_comment` also named the
  comment's text `t` inside its aggregate and outside it, so each `@jsx` comment was its own first: the aggregate's
  text is now its own variable. The same cause as react-practice-a's review found for `@jsxImportSource`.
- `class_component`: isExplicitComponent, a `/** … */` right before the class's target (its export statement, the
  declaration, or a class expression's ESTree grandparent), ending at most a line above it, with an `@extends` or
  `@augments` tag at a line's start naming `React.Component` or `React.PureComponent` and nothing after it up to the
  next tag (doctrine drops a tag with content after the name, `@extends React.Component<P>` or a description line
  after it, and `{React.Component}` is a type, no name). The superclass is read through parentheses, and
  `React[Component]` counts (`superclass`, `pragma_member`). Checked against ESLint on 15 JSDoc forms
  (`us-jsdoc2.jsx`), all matching.
- `create_class`: any argument whose ESTree parent is the call (parentheses aside), of a call or a `new`, the callee
  through parentheses.
- `tsx/react-class`'s `method_key` takes `method_signature` and `abstract_method_signature` (ESTree's
  MethodDefinition and TSAbstractMethodDefinition), and `estree_name` reads a computed key through parentheses and
  comments.

Every react rule reads these, so the whole pack was checked (`ptcheck -- --pack react`, the fixed files over
a5c03d0237, against the same run of 4d492cc30b): every rule's counts are unchanged but react-no-deprecated's, whose
fixture `no-deprecated/invalid-002.jsx` (`/** @jsx Foo */ Foo.renderComponent()`) is now found as the original
finds it (fixtures 1 original-only → 0): its pragma is read. The six rules here keep their counts (samples-react
11, 3, 5, 20, 24, 8 translation-only, all eslint-disable suppressions): no corpus file holds a reviewed case.
