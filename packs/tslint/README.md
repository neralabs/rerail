# Rerail rule pack: TSLint

Ports of TSLint core rules to `rerail.policy/v1`, each with TSLint's default options. One document per rule,
`rules/<rule>.json`, rule id `tslint:<rule>`. Core ships this pack inside its own package until it loads installed
packs, and seeds the rules into the rule catalog **disabled**; enable the ones you want. Every document has
`mode: "steer"`: an enabled rule steers the agent. None uses `block` until block enforcement lands.

Each rule's `scope` covers TypeScript sources (`.ts`, `.tsx`, `.mts`, `.cts`) and leaves out dependency, build-output
and generated directories (`node_modules`, `dist`, `build`, `out`, `.next`, `coverage`, `generated`, ...).

It is checked by `npx vitest run test/engine/catalog/packs.test.ts -t "pack tslint"`, against the
[pack structure and validation contract](../../docs/pack-format.md), and its ports are scored by the TSLint
conformance harness, [`conformance/tslint/`](../../conformance/tslint/README.md) (`npm run test:conformance`).

| Path | What |
|---|---|
| `pack.json` | The pack (`rerail.rule-pack/v1`): each rule's document, title and group |
| `rules/` | The shipped ports: the golden set |
| `conformance.json` | The golden manifest (`rerail-tslint-golden/v1`): per shipped rule, the verdict and true positives the golden gate holds it to, and its notes |
| `divergences.json` | The PASS audit (`rerail-tslint-audit/v1`): for every port that passes, whether it implements TSLint's semantics on real code or where it differs |
| `candidates/` | Ports not shipped (`<rule>.json`), their verdict sidecars (`<rule>.verdict.json`), and every rule's option-variant overrides (`<rule>/<variant>.json`) |
| `licenses/tslint/` | TSLint's licence and `UPSTREAM.md` |

TSLint's own rule tests are not copied here: the harness fetches them at a pinned commit
(`node --import tsx conformance/tslint/cli.ts fetch`, into `tmp/tslint-fixtures/`).

## Provenance

Every document in `rules/` is in the **golden set**. The harness proved it against TSLint's own rule test fixtures,
and the audit classed it as genuine:

- effective PASS: every fixture matches TSLint exactly, or matches once lines that are not valid TypeScript are
  repaired;
- not fixture-fit: it does not pass only because the fixtures miss what it gets wrong;
- not option-baked: it does not hardcode an option list where TSLint's default differs;
- needs no configuration: TSLint's option-less behaviour is meaningful;
- identifier-rename gate clean: it scores identically when the fixtures' own identifiers are renamed.

These documents are what the harness scores. Option-variant overrides used to test the rule's other TSLint options
stay in `candidates/<rule>/<variant>.json`.

## The golden gate

`conformance.json` records, per rule, the expected verdict (`PASS`, or `PASS (repaired fixture)` with the invalid
fixture lines cited), the true positives, and any fixture files outside the shipped scope. Every full conformance run,
the harness's `golden` command and `conformance/tslint/golden-gate.test.ts` fail with a message per rule when a
document here misses its entry. `node --import tsx conformance/tslint/promote.ts --check` keeps `rules/`,
`conformance.json` and the audit in agreement; the unit tests run it.

## Adding a rule

1. Write the port under `candidates/<rule>.json` (plus `candidates/<rule>/<variant>.json` overrides for option
   variants).
2. Iterate with `node --import tsx conformance/tslint/cli.ts --rule <rule>` until it passes, and record a `genuine`
   class for it in `divergences.json`.
3. Run `node --import tsx conformance/tslint/promote.ts <rule>`. It checks the port with the shipped document
   (effective PASS, rename-gate clean), writes `rules/<rule>.json` and the rule's `conformance.json` entry, then
   prints the port default and verdict sidecar to `git rm`.
4. List the rule in `pack.json` with its `document`, `title` and `group`, and add a changeset.

## Attribution

The rule semantics, names and descriptions come from [TSLint](https://github.com/palantir/tslint)
(Copyright 2013 Palantir Technologies, Inc.), licensed under the
[Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0). These documents are Rerail's
re-implementations in its policy DSL; TSLint is archived and not a dependency. TSLint's licence and the record of
what is adapted from it are in [`licenses/tslint/`](licenses/tslint/UPSTREAM.md), and the notice is also in core's
[`THIRD_PARTY_NOTICES`](../../THIRD_PARTY_NOTICES).
