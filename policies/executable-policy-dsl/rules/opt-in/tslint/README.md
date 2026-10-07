# TSLint opt-in pack

Ports of TSLint core rules to `rerail.policy/v1`, each with TSLint's default options. One document per rule,
`<rule>.json`, rule id `tslint:<rule>`. Rerail seeds them into the rule catalog **disabled**; enable the ones you want.
Every document has `mode: "steer"`: an enabled rule steers the agent. None uses `block` until block enforcement lands.

Each rule's `scope` covers TypeScript sources (`.ts`, `.tsx`, `.mts`, `.cts`) and leaves out dependency, build-output
and generated directories (`node_modules`, `dist`, `build`, `out`, `.next`, `coverage`, `generated`, ...).

## Provenance

Every document here is in the **golden set**. The TSLint conformance harness (`node --import tsx conformance/tslint/cli.ts`,
see [`conformance/tslint/README.md`](../../../../../conformance/tslint/README.md)) proved it against TSLint's own
rule test fixtures, and the audit classed it as genuine:

- effective PASS: every fixture matches TSLint exactly, or matches once lines that are not valid TypeScript are
  repaired;
- not fixture-fit: it does not pass only because the fixtures miss what it gets wrong;
- not option-baked: it does not hardcode an option list where TSLint's default differs;
- needs no configuration: TSLint's option-less behaviour is meaningful;
- identifier-rename gate clean: it scores identically when the fixtures' own identifiers are renamed.

These documents are what the harness scores. Option-variant overrides used to test the rule's other TSLint options
stay in `conformance/tslint/rules/<rule>/<variant>.json`.

## The golden gate

`conformance/tslint/golden.json` records, per rule, the expected verdict (`PASS`, or `PASS (repaired fixture)`
with the invalid fixture lines cited), the true positives, and any fixture files outside the shipped scope. Every
full conformance run and the harness's `golden` command fail with a message per rule when a document here misses
its entry. `node --import tsx conformance/tslint/promote.ts --check` keeps this folder, `golden.json` and the audit
in agreement; the unit tests run it.

## Adding a rule

1. Write the port under `conformance/tslint/rules/<rule>.json` (plus `rules/<rule>/<variant>.json` overrides for
   option variants).
2. Iterate with `node --import tsx conformance/tslint/cli.ts --rule <rule>` until it passes, and record a `genuine`
   class for it in `conformance/tslint/audit.ts`.
3. Run `node --import tsx conformance/tslint/promote.ts <rule>`. It checks the port with the shipped document (effective PASS,
   rename-gate clean), writes `<rule>.json` here and the rule's `golden.json` entry, then prints the port default and
   verdict sidecar to `git rm`.

## Attribution

The rule semantics, names and descriptions come from [TSLint](https://github.com/palantir/tslint)
(Copyright 2013 Palantir Technologies, Inc.), licensed under the
[Apache License, Version 2.0](https://www.apache.org/licenses/LICENSE-2.0). These documents are Rerail's
re-implementations in its policy DSL; TSLint is archived and not a dependency. The notice and the full license text
are in [`THIRD_PARTY_NOTICES`](../../../../../THIRD_PARTY_NOTICES).
