# Rerail rule pack: JavaScript and TypeScript (ESLint core and eslint-plugin-unicorn), candidates

The ports in `rules/` here are still being evaluated: scored against the tools' expected findings and reported, never
gated and never shipped. This file is not published; the pack's [README](../README.md) covers the rules it ships.

## Rule details

What each rule matches beyond its one-line description: exact matching, edge cases, upstream settings and known divergences.

### `eslint-no-unused-vars`

Covers variables, parameters, caught errors, imports, types and type parameters. Reported where ESLint reports (the last write from the variable's own function, else its first definition) via `binds` and `resolves.write: "last"`; options filter on those. Also flags names matching an ignore pattern that are in fact used. An unused `/* global */` name with no declaration is reported at the name in the first comment that declares it (first three names of a comment). Annex B: a `var` of a catch parameter's name with an initializer (or a `for-in/of` `var` head) in its catch body writes the parameter in ESLint, which then reports it at that write, not at the parameter; the engine binds that write to the hoisted `var`, so the port does not report the parameter there (and misses ESLint's report at the write).
