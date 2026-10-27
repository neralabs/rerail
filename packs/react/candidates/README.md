# Rerail rule pack: React, candidates

The ports in `rules/` here are still being evaluated: scored and reported against the tools' recordings, never gated
and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `react-button-has-type`

Port of eslint-plugin-react 7.37.5 react/button-has-type with button, submit and reset allowed. Reports a `<button>` with no `type` attribute (looked up case-insensitively, as jsx-ast-utils getProp does) or whose first `type` value is not a literal button/submit/reset. Each leaf of a conditional `type` is checked, and a non-literal leaf is reported at itself. The same applies to `React.createElement('button', props)` with the default pragma.

### `react-hooks-exhaustive-deps`

Port of eslint-plugin-react-hooks 7.1.1 react-hooks/exhaustive-deps, missing-dependency report only (no unnecessary/duplicate-dependency reports). Checks the dependency array literal of useEffect, useLayoutEffect, useInsertionEffect, useCallback and useMemo (bare or as `React.*`) when the inline callback reads a name bound in the calling function (outside the callback) that no dependency's root names. useState/useReducer setters and dispatchers, useRef, useEffectEvent and useTransition results count as stable and need not be listed.

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

### `react-react-in-jsx-scope`

Port of eslint-plugin-react 7.37.5 react/react-in-jsx-scope (pragma React). A JSX opening element or fragment is reported when no `React` binding is visible: none declared at the top level (import, var/let/const, function), in an enclosing block's own declarations, or as an enclosing function's parameter.

### `react-self-closing-comp`

Port of eslint-plugin-react 7.37.5 react/self-closing-comp (component: true, html: true). Reports an element with a closing tag and no children except whitespace that contains a line break (a same-line space is text and is not reported). Namespaced tags only when lower-case (DOM).

### `react-void-dom-elements-no-children`

Port of eslint-plugin-react 7.37.5 react/void-dom-elements-no-children. Void elements: area, base, br, col, embed, hr, img, input, keygen, link, menuitem, meta, param, source, track, wbr. Reports children, a `children` attribute or a `dangerouslySetInnerHTML` attribute (one report each); the same through createElement with an object-literal props argument.
