# react-04: six eslint-plugin-react rules against their originals

Rules: `react-no-find-dom-node`, `react-no-render-return-value`, `react-no-string-refs`,
`react-no-unescaped-entities`, `react-no-unknown-property`, `react-no-unsafe`.

Ground truth: ESLint 9 with eslint-plugin-react 7.37.5, `@typescript-eslint/parser` with JSX on for every file,
`settings.react.version` "19.0.0" (the recording assumes the latest React), each rule at "error", run over the
`samples-react` corpus and the rules' fixtures on a fleet box.

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

## Library changes

None needed.
