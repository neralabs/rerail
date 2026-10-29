# Rerail rule pack: React, candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `react-hooks-exhaustive-deps`

Port of eslint-plugin-react-hooks 7.1.1 react-hooks/exhaustive-deps, missing-dependency report only (no unnecessary/duplicate-dependency reports). Checks the dependency array literal of useEffect, useLayoutEffect, useInsertionEffect, useCallback and useMemo (bare or as `React.*`) when the inline callback reads a name bound in the calling function (outside the callback) that no dependency's root names. useState/useReducer setters and dispatchers, useRef, useEffectEvent and useTransition results count as stable and need not be listed.

### `react-react-in-jsx-scope`

Port of eslint-plugin-react 7.37.5 react/react-in-jsx-scope (pragma React). A JSX opening element or fragment is reported when no `React` binding is visible: none declared at the top level (import, var/let/const, function), in an enclosing block's own declarations, or as an enclosing function's parameter.
