---
"@neralabs/rerail-pack-react": patch
---

React pack: ships its first 38 rules, 30 from eslint-plugin-react 7.37.5 and 8 from eslint-plugin-react-hooks 7.1.1, each checked against the plugins on their own test cases and on the pack's real-code samples, most of them from the React test set: 738 real files from 57 open-source repositories, each at the commit that introduced a violation, 631 hand-fixed copies, and 99 files whose own `eslint-disable` comment silences the rule.

19 start on, as the plugins' recommended configurations have them: `react-jsx-key`, `react-jsx-no-comment-textnodes`, `react-jsx-no-duplicate-props`, `react-jsx-no-target-blank`, `react-jsx-no-undef`, `react-no-children-prop`, `react-no-danger-with-children`, `react-no-deprecated`, `react-no-direct-mutation-state`, `react-no-find-dom-node`, `react-no-is-mounted`, `react-no-render-return-value`, `react-no-string-refs`, `react-no-unescaped-entities`, `react-no-unknown-property`, `react-hooks-rules-of-hooks`, `react-hooks-exhaustive-deps` (in log mode, as the plugin warns), and the React Compiler checks `react-hooks-globals` and `react-hooks-set-state-in-effect`.

19 start off. Fifteen are off in the plugins' recommended configurations: `react-button-has-type`, `react-iframe-missing-sandbox`, `react-jsx-no-constructed-context-values`, `react-jsx-no-leaked-render`, `react-jsx-no-script-url`, `react-no-access-state-in-setstate`, `react-no-array-index-key`, `react-no-danger`, `react-no-did-mount-set-state`, `react-no-did-update-set-state`, `react-no-object-type-as-default-prop`, `react-no-unsafe`, `react-no-unstable-nested-components`, `react-self-closing-comp` and `react-void-dom-elements-no-children`. Four React Compiler checks the plugin recommends start off as experimental, since its test cases never report them: `react-hooks-error-boundaries`, `react-hooks-purity`, `react-hooks-set-state-in-render` and `react-hooks-static-components`.

`react-react-in-jsx-scope`, `react-hooks-immutability` and `react-hooks-refs` stay candidates: the first reports every file on the automatic JSX runtime, and the other two still miss findings that need the React Compiler's data-flow analysis.
