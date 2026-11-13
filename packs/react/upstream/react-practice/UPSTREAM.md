# React practice rules: cases Rerail wrote

The React pack's own practice rules (`rules/react-practice-*.json`, listed as `own` in `conformance.json`) port no
linter, so this directory has no upstream tool, generator or `gates.yaml`. Its test data is one case list per rule:
the code each rule must catch and must pass, as a `rerail rules test` draft's `cases` holds them, run by
`test/cli/react-practice-rules.test.ts`. The cases are Rerail's (Apache-2.0, the pack's licence), written for these
rules, so nothing goes under `licenses/`. Paths are relative to the pack directory.

| File | Origin |
|---|---|
| `upstream/react-practice/react-practice-cache-update-in-mutate-call.cases.json` | written by Rerail |
| `upstream/react-practice/react-practice-callback-for-dom-only.cases.json` | written by Rerail |
| `upstream/react-practice/react-practice-fresh-dependency.cases.json` | written by Rerail |
| `upstream/react-practice/react-practice-hoist-static-value.cases.json` | written by Rerail |
| `upstream/react-practice/react-practice-lifecycle-hook.cases.json` | written by Rerail |
| `upstream/react-practice/react-practice-memo-trivial-expression.cases.json` | written by Rerail |
| `upstream/react-practice/react-practice-mounted-ref-never-reset.cases.json` | written by Rerail |
| `upstream/react-practice/react-practice-request-beside-data-library.cases.json` | written by Rerail |
| `upstream/react-practice/react-practice-selector-returns-new-collection.cases.json` | written by Rerail |
| `upstream/react-practice/react-practice-side-effect-in-render.cases.json` | written by Rerail |
| `upstream/react-practice/react-practice-then-without-stale-guard.cases.json` | written by Rerail |
