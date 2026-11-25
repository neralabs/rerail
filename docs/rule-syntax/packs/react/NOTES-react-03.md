# React batch react-03

Rules: `react-jsx-no-target-blank`, `react-jsx-no-undef`, `react-no-access-state-in-setstate`, `react-no-danger`,
`react-no-did-mount-set-state`, `react-no-did-update-set-state`.

Ground truth: ESLint 9.39.5 with eslint-plugin-react 7.37.5 at its defaults (no settings, so no React version), run
on every file of `samples-react` and on the six rules' recorded fixtures, beside the translations
(`rerail-engine run-policies`) and the originals (the product's path). On both corpora the originals agree with
ESLint, bar two `react-no-danger` lines (below). Every difference left between translation and original is one
engine gap, `run-policies` applying no suppression comment; the fixes below removed the rest.

## Fixed in the translations

- **react-jsx-no-target-blank: `value()` and negation.** Five valid fixtures were reported, e.g. valid-054
  `<a href={href} target={isExternal ? undefined : "_blank"} rel={isExternal ? "noreferrer" : "noopener noreferrer"} />`.
  The rule's logic was right; the engine's was not. A relation whose body reads `value(b, "_blank")` gives nodes
  on which a later negation is wrong. Minimal repro, on that fixture:

  ```
  .decl ve(v: node)
  ve(v) :- kind(c, "ternary_expression"), field(c, "alternative", b), value(b, "_blank"), field(c, "consequence", v).
  dbg(v) :- ve(v), !kind(v, "string").     // holds for the string "_self" in target={isSelf ? "_self" : "_blank"}
  ```

  With `str(b, "_blank")` (or `text(b, "\"_blank\"")`) in place of `value`, the negation is right. Here it made
  `rel_unknown` hold for every paired ternary rel (`!kind(v, "string")` on a string branch), so no rel was
  secure. The rule now reads string literals with `str` (four places). Engine gap: see below.
- **react-jsx-no-undef: `import { default as X }`.** `lepton …/snippet/index.js:291` `<GistEditorForm>` with
  `import { default as GistEditorForm, UPDATE_GIST } from '../gistEditorForm'` was reported. The tape keeps no
  node for `default`, so the specifier has an `alias` and no `name`, and names.rerail's `import_binding` (which
  needs the name) binds nothing. The rule adds that binding itself (one clause). ESLint: not reported.
- **react-no-access-state-in-setstate: source order.** 39 original-only lines (excalidraw, perseus, react-select,
  slate, …), e.g. `let toOpen = this.state.isOpen || …; this.setState({ isOpen: toOpen })`. The rule ordered the
  read before the use with core's `earlier`, which has no rows on JavaScript or TypeScript (NOTES-next-01). It now
  includes `tsx/next-01.rerail` and uses `src_before` (start offsets). The nodes compared are a read and a later
  use, or a pattern and a later use, which never nest, so start order is the plugin's visit order.
- **react-no-access-state-in-setstate: destructuring outside a component.** Fixture invalid-003,
  `React.createClass({ onClick: function() { var {state, ...rest} = this; this.setState({value: state.value + 1}) } })`.
  The plugin's ObjectPattern and Identifier visitors check no component (only its MemberExpression visitor does),
  and `React.createClass` is not a component under the default `createClass` setting (`createReactClass`). The
  rule required `comp_owner` for the pattern; it no longer does. ESLint: reported at line 4. Checked ad hoc: a
  plain `class Plain { m() { var {state} = this; this.setState({v: state.v}); } }` is reported by ESLint and now
  by the translation.

## Left: suppression comments (right: original)

`rerail-engine run-policies` returns every diagnostic with no suppression applied or marked
(`native/src/code_rule_dsl/tools.rs`, `run_policies`, has no suppression step), though each rule declares the
JSON's `suppress` lines. Repro: `// eslint-disable-next-line react/no-danger` above
`<div dangerouslySetInnerHTML={{__html: x}} />` gives one diagnostic, line 2. ESLint and the original honour the
comment. Every translation-only key left is such a line:

| Rule | Lines | Comments |
|---|---|---|
| react-jsx-no-target-blank | excalidraw DropdownMenuItemLink.tsx:35 | `eslint-disable-next-line` |
| react-jsx-no-undef | ant-design alert index.test.js:64 | `eslint-disable-next-line` |
| react-no-access-state-in-setstate | superset CategoricalDeckGLContainer.jsx:105, 203, 221; Multi.jsx:110; docusaurus IdealImage index.js:314, 321 | file-level `/* eslint-disable … */`, `eslint-disable-line` |
| react-no-danger | 6 lines (docusaurus Mermaid, jitsi, mui ClassesList ×2, react-admin, superset html.tsx); ant-design CodePreview.tsx:83; polar layout.tsx:60 | `eslint-disable-next-line` (also as `{/* */}` in JSX); `biome-ignore lint/security/noDangerouslySetInnerHtml`; `oxlint-disable-next-line react/no-danger` |
| react-no-did-mount-set-state | 5 lines (ant-design, blueprint, material-ui, perseus, superset) | `eslint-disable-line`, `-next-line`, block `eslint-disable` |
| react-no-did-update-set-state | 15 lines (react-select ×11, ant-design, jitsi, perseus, superset) | `eslint-disable-line`, `-next-line` |

ESLint itself ignores `biome-ignore` and `oxlint-disable` comments, so on the last two no-danger lines ESLint
agrees with the translation; the original honours the rule's biome and oxlint suppressions, as the product does.
The original is the product's behaviour, so it is counted right.

## Final state

react-no-danger, react-no-did-mount-set-state and react-no-did-update-set-state are unchanged: their only
differences were suppressed lines. Last check, all six rules (`ptcheck … -- --pack react --rules …`, exit 0):

| Rule | examples (both/o/t) | fixtures (both/o/t) | samples-react (both/o/t) | Was (fixtures; samples-react o/t) |
|---|---|---|---|---|
| react-jsx-no-target-blank | 1/0/0 | 38/0/0 | 48/0/1 | 0/5; 0/1 |
| react-jsx-no-undef | 4/0/0 | 332/0/0 | 63/0/1 | 0/0; 0/3 |
| react-no-access-state-in-setstate | 1/0/0 | 2/0/0 | 159/0/6 | 1/0; 39/2 |
| react-no-danger | 3/0/0 | 7/0/0 | 63/0/8 | unchanged |
| react-no-did-mount-set-state | 1/0/0 | 14/0/0 | 50/0/5 | unchanged |
| react-no-did-update-set-state | 1/0/0 | 14/0/0 | 49/0/15 | unchanged |

access-state's translation-only count rose from 2 to 6: with source order working, four more suppressed reads
(superset CategoricalDeckGLContainer.jsx:203, 221; docusaurus IdealImage index.js:314, 321) are found.

## Engine gaps (proposed)

1. **run-policies applies no suppression.** It should apply (or mark) each policy's `suppress` comments as the
   product path does; the same gap shows on the Bandit translations' nosec fixtures (NOTES-misc-untranslated).
2. **`value()` poisons later negations.** A relation whose body reads `value(n, "const")` (`value(n, s), s = "…"`
   alike) yields nodes on which `!kind(v, …)` or `!rel(v)` succeeds when the positive atom also does. `str` does
   not. Until fixed, rules should prefer `str` for string literals where a negation follows.
3. **`earlier` is empty on JavaScript and TypeScript** (already in NOTES-next-01). This batch uses next-01's
   `src_before`; when `visit_index` is filled for these documents, the access-state rule says `earlier` again.

## Library changes needed

- `lib/tsx/names.rerail`: `import_binding` should bind an `import_specifier` with an alias and no name
  (`import { default as X }`) as `(sp, X, m, "default")`. Then jsx-no-undef's own clause goes.
- `lib/tsx/next-01.rerail`'s `src_before` is general source order, not a Next.js concept; it belongs in a tsx
  library both packs read (or goes with the engine fix).
