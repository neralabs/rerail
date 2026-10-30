# @neralabs/rerail-pack-next

## 0.1.5

### Patch Changes

- 61cda45: Each pack's `covers`, the line the dashboard shows under the pack's name, says in one sentence what the pack's rules are for and where they come from. It no longer counts rules or says how many start on: the dashboard shows both beside it, and the counts went stale whenever a rule shipped.
- 1e7042b: README: the note that deriving pack test data is paused is gone, since derive runs again; no rule changes.
- c8352a7: README, `pack.json` descriptions and licence notes: the conformance check is the one gate a port passes to ship, the ECMAScript packs' tests carry rename variants that upstream labels (`holds-under-rename`), and new test data is published through a tool-pack's `publish/` directory; no rule changes.
- c8352a7: Licence notes: each `licenses/<project>/UPSTREAM.md` records its upstream repository per repo (source, the path patterns taken, what is not copied) instead of one row per copied test file; no rule changes.
- c8352a7: README and licence notes: the ports are scored on labelled real-code samples instead of pinned corpora; no rule changes.

## 0.1.4

### Patch Changes

- 9c30a1e: Next.js pack: the 15 rules `@next/eslint-plugin-next`'s recommended configuration sets to `warn` now start in `log` mode, recorded but not steered: `no-img-element`, `no-head-element`, `no-sync-scripts`, `no-css-tags`, `google-font-display`, `google-font-preconnect`, `next-script-for-ga`, `no-unwanted-polyfillio`, `no-styled-jsx-in-document`, `no-title-in-document-head`, `no-async-client-component`, `no-before-interactive-script-outside-document`, `no-typos`, `no-page-custom-font` and `no-location-assign-relative-destination`. The six it sets to `error` still steer.
- 7392aea: README and licence notes: the pack's test data now lives outside the package; no rule changes.
- d0dd309: README: internal ticket references removed; no rule changes.
- 211e8b5: The note on the port readiness recording in each `licenses/*/UPSTREAM.md` names the upstream licence file kept beside it (`LICENSE`) by its own name.

## 0.1.3

### Patch Changes

- e06db22: Next.js pack: messages whose rewrite only changed the voice are back to their original wording; in an eval with a coding agent, the original and rewritten messages steered it equally well.
- e06db22: Next.js pack: every rule description now says in one plain sentence what the rule wants; the exact matching detail moved to the pack README.
- e06db22: Next.js pack: every violation message now says in one sentence what to change in the flagged code, without tool names or rule codes.

## 0.1.2

### Patch Changes

- 0ab3ced: Next pack: `next-no-img-element`'s message now says what switching to `next/image` needs (`width` and `height`, or `fill`, unless `src` is a static import; a matching `images.remotePatterns` entry for a remote `src`) and that `// eslint-disable-next-line @next/next/no-img-element -- <reason>` keeps `<img>`, for example for images from arbitrary hosts. What it reports is unchanged.
- d1e67d7: Next.js pack: `google-font-display`, `no-head-element`, `no-location-assign-relative-destination` and `no-page-custom-font` pass every rule gate and ship, all on; 21 rules ship and none remain candidates.
- ac01449: Next probes: the app-router, pages-router, route-handler, segment-config, middleware and client-hook probes read `app/`, `src/app/`, `pages/` and `middleware` paths relative to the nearest `package.json` that depends on `next`, so a Next app in a subfolder (`apps/web/app/page.tsx`) is checked and an `app/` folder of a package without `next` is not.
- ac01449: Next probes: the client-hook probe reports a hook only in a module of the app's server module graph (`file.package.serverGraph`), so a hook module that only a `"use client"` module imports, or that nothing imports, is no longer reported, and one a server page reaches through a `tsconfig.json` alias is.
- 3049ced: The match definition names its npm dependencies in the new form, `{ "dependencies": { "ecosystem": "npm", "names": [...] } }`, which core reads alongside PyPI and Composer dependencies. Which repositories the pack fits is unchanged.

## 0.1.1

### Patch Changes

- 93bce5e: Recordings: two cases Rerail wrote, an `<img>` and a `<script>` whose `src` holds a bare `&`, now scored against the plugin.
- 37c902a: Next.js pack: every rule honours Oxlint 1.88.0's suppression comments naming it (`// oxlint-disable-next-line no-img-element` and the other `oxlint-disable` forms), as well as ESLint's. The 9 rules Biome 2.5.15 has a rule the same as honour `biome-ignore` comments naming it (`// biome-ignore lint/performance/noImgElement: reason` and the other `biome-ignore` forms), as do 2 candidates.
- fbb6104: First release: 17 @next/eslint-plugin-next 16.3.8 rules ported to Rerail code rules.
- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- 740cf08: One rule id style in every pack, before any pack is published: lowercase letters, digits and dashes, the same string as the rule's document file name and its port's id in `conformance.json`. JavaScript's `eslint:<rule>` and `unicorn:<rule>` ids become `eslint-<rule>` and `unicorn-<rule>`, with their documents renamed to match; TypeScript's `typescript-eslint:<rule>` become `typescript-eslint-<rule>`; Python's `python-conformance:<tool>-<rule>` become `<tool>-<rule>`; React's `react:no-array-index-key` becomes `react-no-array-index-key`; Next's probe documents are named for their ids; the complexity rules become `complexity-cyclomatic-<language>`. TypeScript's `no-explicit-any` candidate is converted from `tempo.policy/v1` to `rerail.policy/v1` and moves to `candidates/rules/`. The JavaScript and TypeScript packs say how many of their rules start on.
