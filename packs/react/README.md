# Rerail rule pack: React

18 eslint-plugin-react 7.37.5 and eslint-plugin-react-hooks 7.1.1 rules ported to Rerail: 14 start on, as the plugins'
recommended configurations turn them on, and `react-button-has-type`, `react-no-danger`, `react-self-closing-comp`
and `react-void-dom-elements-no-children` start off until turned on. 3 more ports are candidates
([candidates/README.md](candidates/README.md)). A port starts in `candidates/rules/` and moves to `rules/` and
`pack.json` once it passes the conformance check ([conformance/README.md](../../conformance/README.md#the-gate)), which
CI runs on every port. Every port is scored against the plugins' own findings on the
plugins' test cases and on their rename variants (`holds-under-rename`), and against the labelled lines of the plugins'
real-code samples ([docs/upstream.md](../../docs/upstream.md#samples)), through the engine's production workspace
path. The conformance check (`--check`, run in CI) scores the test cases and the samples: it gates the ports in
`rules/` and reports the candidates without gating them.

Every rule runs on JavaScript and TypeScript files whose JSX runtime is React or not known; a file whose JSX belongs to
another library is skipped. The runtime is the file's `@jsxImportSource` pragma when it names React, Emotion
(`@emotion/react`) or Theme UI (React) or Preact, Solid, Vue, Stencil, Hono, Qwik, Inferno, Mithril or Hyperapp (another
library), or its `@jsx` pragma when that names `React`; else an `h` or `createElement` imported from Stencil, Preact,
Vue, Solid, Hyperapp or Mithril (another library); else its nearest `package.json`: React when it depends on react,
react-dom, react-native or next and on none of vue, @stencil/core, preact, solid-js, @builder.io/qwik, inferno or
mithril, another library in the reverse case, unknown otherwise. `react-in-jsx-scope` also skips files whose `@jsx`
pragma names another factory (Emotion's `jsx`, for one): the plugin then asks for that factory in scope, which the
port does not check.

The pack follows the [pack structure and validation contract](../../docs/pack-format.md): it is checked by
`npx vitest run test/engine/catalog/packs.test.ts -t "pack react"`.

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): its match definition, and the 18 rules it ships |
| `rules/` | The shipped rules' code-rule documents, written by `candidates/build.mjs` from `candidates/specs.mjs` |
| `conformance.json` | Every port (shipped under `rules/`, or a candidate under `candidates/rules/`): its upstream tool and rule, how findings compare and the capabilities it exercises; the pinned tools. Written by `candidates/build.mjs` |
| `upstream/<tool>/` | The plugin's copied tests (`tests/`), the cases extracted from them (`fixtures/`, their rename variants under `fixtures/renamed/`), its expected findings (`fixtures.expected.json`), the generator (`generate.mjs`, sharing `upstream/common.mjs` and `scripts/pack-data/eslint-expected.mjs`), and the lock naming its real-code samples (`samples.lock`, fetched by `npm run fixtures:pull`) with their sources and licences (`SAMPLES.md`); `UPSTREAM.md` lists the fixtures Rerail wrote beside them (`common.mjs` `AUTHORED`): files of other JSX libraries, which the plugins report and the ports skip, and React files whose JSX goes through Emotion, which both report |
| `divergences.json` | Where the ports and the expected findings disagree by design: the plugins' findings on the authored files of other JSX libraries |
| `licenses/<project>/` | The upstream licence and `UPSTREAM.md`, per upstream repository: its source, the patterns of the files taken and how (each file's provenance is `provenance.json` in the tool's test-data archive) |
| `candidates/` | Ports still being evaluated, scored but never shipped: the code-rule documents (`rules/`, written by `build.mjs` from `specs.mjs`, except the hand-written `react-no-array-index-key.json`); `probes/` holds the hand-labelled React fact probes |
| `candidates/README.md` | What each candidate is waiting on and its rule details; not published |

Taking a new snapshot is a maintainer deriving the pack's test data from its pins and publishing it
([docs/upstream.md](../../docs/upstream.md#take-a-new-snapshot)); a pull request carries it only as a publish request
under `upstream/<tool>/publish/`. The recipe (`scripts/pack-data/recipes/react.ts`) runs each plugin's `generate.mjs`
against the pinned tools (eslint 9.39.5, @typescript-eslint/parser 8.70.1, typescript 5.9.3) and the plugins' upstream
checkouts. The fixtures Rerail wrote (`UPSTREAM.md`) are authored: a change to them is staged into `publish/` with the
derived data.

```sh
npm run fixtures:derive -- --pack react                                     # both plugins
npm run fixtures:derive -- --pack react --tool eslint-plugin-react-hooks    # one
npm run fixtures:stage -- --pack react --tool eslint-plugin-react-hooks     # copy what changed into upstream/eslint-plugin-react-hooks/publish/
```

Rewriting the rule documents and `conformance.json` after editing `candidates/specs.mjs`, then scoring one rule with
every false positive and false negative printed:

```sh
node candidates/build.mjs
node --import tsx conformance/languages/cli.ts --pack react --rule react-jsx-key --fixtures
```

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `react-button-has-type`

Port of eslint-plugin-react 7.37.5 react/button-has-type with button, submit and reset allowed. Reports a `<button>` with no `type` attribute (looked up case-insensitively, as jsx-ast-utils getProp does) or whose first `type` value is not a literal button/submit/reset. Each leaf of a conditional `type` is checked, and a non-literal leaf is reported at itself. The same applies to `React.createElement('button', props)` with the default pragma.

### `react-hooks-rules-of-hooks`

Port of eslint-plugin-react-hooks 7.1.1 react-hooks/rules-of-hooks. A hook call (`use`, `useX`, `Namespace.useX`) is reported at its callee when it runs, in a component or hook (a function named `X…`/`useX`, or the callback of memo/forwardRef), conditionally (if/conditional/logical/switch/catch/default branch, or after an earlier return); in a class method or field; in another named function; at the top level; in an anonymous callback inside a component or hook; in a loop; in an async component or hook; and `use` inside try/catch. ESLint's code-path analysis is approximated syntactically.

### `react-jsx-key`

Port of eslint-plugin-react 7.37.5 react/jsx-key, default options. Reports an element (not a fragment) without `key` that is an array literal's element, or that a `.map(fn)` / `.from(x, fn)` callback returns: the arrow's body, a branch of its conditional body, the right side of its logical body, or a return statement directly in the callback body or in nested if/else branches. Never reported inside `React.Children.toArray(...)` / `Children.toArray(...)`.

### `react-jsx-no-comment-textnodes`

Port of eslint-plugin-react 7.37.5 react/jsx-no-comment-textnodes: JSX text with a line that starts (after whitespace) with `//` or `/*`.

### `react-jsx-no-duplicate-props`

Port of eslint-plugin-react 7.37.5 react/jsx-no-duplicate-props, case-sensitive. Namespaced attribute names and spreads are skipped.

### `react-jsx-no-target-blank`

Port of eslint-plugin-react 7.37.5 react/jsx-no-target-blank with default options (links only, enforceDynamicLinks always, allowReferrer false). Reports an `<a>` whose last `target` is possibly `_blank` (a string, or a conditional with a `_blank` branch), with an external last `href` string (`scheme:` or `//`) or any `href` expression, and whose last `rel` does not give `noreferrer` on every value it may take (a string, the first quasi of a template, both branches of a conditional, or, when target and rel test the same name, the branch on which target is `_blank`).

### `react-jsx-no-undef`

Port of eslint-plugin-react 7.37.5 react/jsx-no-undef (allowGlobals: false, module code). Checks a non-DOM JSX tag name, or the root of a member tag name, against file-local bindings; globals, even configured ones, do not count in a module.

### `react-no-children-prop`

Port of eslint-plugin-react 7.37.5 react/no-children-prop (allowFunctions: false): a `children` JSX attribute, and a createElement call (default pragma) whose second argument is an object literal with a `children` key.

### `react-no-danger`

Port of eslint-plugin-react 7.37.5 react/no-danger, default options. Only DOM elements (tag name starting lower-case, per jsx-ast-utils elementType) are checked; custom components receiving the prop are not reported.

### `react-no-danger-with-children`

Port of eslint-plugin-react 7.37.5 react/no-danger-with-children. Children means a first child that is not a line break, or a `children` prop. Props are also read through `{...name}` spreads of a variable initialised with an object literal. For `x.createElement(type, props, ...)`, props may be an object literal or a variable initialised with one, and children is a third argument or a `children` prop.

### `react-no-direct-mutation-state`

Port of eslint-plugin-react 7.37.5 react/no-direct-mutation-state. Reports an assignment or update whose target is a member chain rooted at `this.state`, inside a class component (nearest class extends Component / PureComponent / React.Component / React.PureComponent) or a function that is a property of the object passed to createReactClass. In a constructor it is reported only when inside a call. Reported at `this`. Function components (the plugin's Components.detect stateless detection) are not modelled.

### `react-no-find-dom-node`

Port of eslint-plugin-react 7.37.5 react/no-find-dom-node: any call whose callee is `findDOMNode` or a member `.findDOMNode`.

### `react-no-is-mounted`

Port of eslint-plugin-react 7.37.5 react/no-is-mounted: `this.isMounted()` inside an object property or a method (ESTree Property / MethodDefinition ancestor).

### `react-no-render-return-value`

Port of eslint-plugin-react 7.37.5 react/no-render-return-value (latest React). The value counts as used when the call's parent (parentheses aside) is a declarator, a property, a return, an arrow body or an assignment.

### `react-no-string-refs`

Port of eslint-plugin-react 7.37.5 react/no-string-refs (latest React, noTemplateLiterals: false): a `ref` attribute whose value is a string literal, bare or in braces. Template literals are not reported. From React 18.3 on the plugin no longer reports `this.refs` reads, so neither does the port.

### `react-no-unescaped-entities`

Port of eslint-plugin-react 7.37.5 react/no-unescaped-entities (forbids >, ", ', }). One finding per such character, but only on the first line of a JSX text node (the k-th check holds where that line has at least k of them; ESLint reports each character at its own column). Characters on a text node's later lines are not reported, because a finding is anchored at a node and a text node has no node per line.

### `react-self-closing-comp`

Port of eslint-plugin-react 7.37.5 react/self-closing-comp (component: true, html: true). Reports an element with a closing tag and no children except whitespace that contains a line break (a same-line space is text and is not reported). Namespaced tags only when lower-case (DOM).

### `react-void-dom-elements-no-children`

Port of eslint-plugin-react 7.37.5 react/void-dom-elements-no-children. Void elements: area, base, br, col, embed, hr, img, input, keygen, link, menuitem, meta, param, source, track, wbr. Reports children, a `children` attribute or a `dangerouslySetInnerHTML` attribute (one report each); the same through createElement with an object-literal props argument.
