# react-tsc-01: six eslint-plugin-react rules against their originals

Rules: `react-button-has-type`, `react-jsx-no-constructed-context-values`, `react-no-array-index-key`,
`react-no-children-prop`, `react-no-direct-mutation-state`, `react-no-unstable-nested-components`.

Ground truth: ESLint 9.39.5 with eslint-plugin-react 7.37.5, `typescript-eslint` 8.74's parser with JSX on for every
file, `settings.react.version` "19.0.0", the six rules at "error", over a copy of the `samples-react` corpus on a
fleet box, run twice: with inline config (as the original) and with `linterOptions.noInlineConfig: true` (no
`eslint-disable` comment honoured, as `run-policies` today). The translations ran on the same copy through
`rerail-engine run-policies` with this branch's library.

## Results

| Rule | Status | Difference left |
|---|---|---|
| react-button-has-type | different | samples-react: 6 translation-only, all silenced by a suppression comment |
| react-jsx-no-constructed-context-values | different | fixed 1 + 1 (below); samples-react: 4 translation-only, same cause |
| react-no-array-index-key | different | samples-react: 11 translation-only, same cause (4 of them by oxlint's reading, below) |
| react-no-children-prop | different | samples-react: 7 translation-only, same cause |
| react-no-direct-mutation-state | different | samples-react: 1 translation-only, same cause |
| react-no-unstable-nested-components | different | samples-react: 4 translation-only, same cause |

Fixtures and examples: no difference for any rule.

## The remaining difference: run-policies does not apply suppression comments

Every translation-only key left is a finding a disable comment for the rule silences, e.g.

    // eslint-disable-next-line react/button-has-type           ghost …/koenig-lexical/src/components/ui/Button.jsx:7
    // eslint-disable-next-line react/no-children-prop          mantine …/InlineDateTimePicker.tsx:388, :449
    {/* eslint-disable-line react/no-children-prop */}          react-admin …/ReferenceFieldController.spec.tsx:26
    // eslint-disable-next-line react/no-direct-mutation-state  excalidraw src/index.tsx:233

Each translation declares the JSON's `suppress eslint/oxlint/biome` lines, but `rerail-engine run-policies` returns
its diagnostics with no suppression applied, so the suite counts them (the gap NOTES-react-04.md describes). Proof:
ESLint with `noInlineConfig` reports exactly the translations' (path, line) keys plus nothing else, for all six
rules; with inline config on, ESLint's keys are the translations' minus these. Recorded with right = original.

`react-no-array-index-key`: 4 of its 11 keys sit under `// eslint-disable-next-line @eslint-react/no-array-index-key`
(streamlit `Particles.tsx` ×3, and one widget file). ESLint does not honour that comment for `react/no-array-index-key`
(another plugin's rule) and reports them; the original does not, because the product's oxlint suppression
(`native/src/code_rule_dsl/suppress/oxlint.rs`) reads a directive's names without their plugin, as oxlint does, so
`@eslint-react/no-array-index-key` names the JSON's `oxlint "no-array-index-key"`. That is the product's documented
reading, and the translation declares the same `suppress oxlint` line, so once run-policies applies suppressions
these keys go too. Still recorded as right = original.

## react-jsx-no-constructed-context-values: provider lookup fixed

The rule's `isReactContext` takes the tag's name, walks up the scope chain to the first scope that declares it, and
asks whether that variable's first definition is a `VariableDeclarator` whose init calls `createContext` or
`React.createContext`. The translation had looked for any value declaration of the name in an enclosing scope **or
in any program**, a declarator only:

- `kind(sc, "program")` matched the program of every file in the run, so a `const MyContext = createContext()` in
  one file made `<MyContext value={{}}>` a provider in another. Fixture `valid-017.jsx` (`function Component({
  MyContext }) { return <MyContext value={{ foo: "bar" }} />; }`) was reported; ESLint does not (a parameter is not a
  declarator, and the parameter hides any outer name).
- `const [RootPropsProvider, useRootProps] = createContext(…)` (chakra-ui `apps/compositions/src/ui/pagination.tsx:56`)
  was missed: tsx/names' site for a destructured name is the pattern's identifier, not the declarator, while ESLint's
  `def.node` is the `VariableDeclarator`. ESLint reports it.

Now `context_def` takes the innermost scope around the tag that declares the name (`max` of `ast_depth`) and its first
definition by start byte, and the provider check maps a pattern name to its declarator with `pattern_name`. Checked
against ESLint on ad-hoc cases, all matching by line:

    const Top = createContext(null);  <Top value={{ a: 1 }} />                   reported
    const [Destr] = createContext(null);  <Destr value={{ a: 1 }} />             reported
    const { Provider: ObjP } = React.createContext(null);  <ObjP value={[1]} />  reported
    function C({ Top }) { return <Top value={{ a: 1 }} />; }                     not: a parameter
    function D() { const Top = Imported; return <Top value={{ a: 1 }} />; }      not: the inner Top
    import { Imported } …  <Imported value={{ a: 1 }} />                         not: an import
    if (x) { const Inner = createContext(); return <Inner value={() => 1} />; }  reported
    valid-017's shape with `const MyContext = React.createContext()` in another file   not

Not modelled: typescript-eslint's scope manager keeps a type and a value of one name as one variable, so `type Ctx
= …; const Ctx = createContext()` makes the type the first definition and ESLint answers false; the translation
looks at values only. Not seen in the corpora.

## Library changes

None needed.
