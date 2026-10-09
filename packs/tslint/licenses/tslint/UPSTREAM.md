# TSLint 285fc1d

Upstream: [palantir/tslint](https://github.com/palantir/tslint/tree/285fc1db18d1fd24680d6a2282c6445abf1566ee) (commit `285fc1db18d1fd24680d6a2282c6445abf1566ee`, the archived repository's last master). License: Apache-2.0 (Copyright 2013 Palantir Technologies, Inc.); [`LICENSE`](LICENSE) is the upstream license file, copied verbatim at that commit. TSLint ships no NOTICE file.

No TSLint file is copied into this pack. The paths listed below are Rerail's re-implementations of TSLint's core rules
in the code-rule DSL, each with TSLint's default options (`rules/`) or with one of the option sets TSLint's tests use
(`candidates/<rule>/<variant>.json`). Rule names and the rule descriptions are taken from TSLint's rule metadata
(`src/rules/<camelName>Rule.ts`), so the documents are annotated in `REUSE.toml` with Palantir's copyright as well as
Nera's, under Apache-2.0. Every other file in this pack is Rerail's, under the Apache License 2.0 (the pack's
`LICENSE`).

TSLint's own rule tests (`test/rules/**/*.lint`) are not copied: the harness in `conformance/tslint/` fetches them at
the commit above into `tmp/tslint-fixtures/` (`node --import tsx conformance/tslint/cli.ts fetch`).

Paths are relative to the pack directory, `packs/tslint/`.

| File | Upstream path | Copy |
|---|---|---|
| `rules/` | `src/rules/*Rule.ts` (rule names, descriptions, default options) | adapted |
| `candidates/` | `src/rules/*Rule.ts` (rule names, descriptions, options of `test/rules/**/tslint.json`) | adapted |
