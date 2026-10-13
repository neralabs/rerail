# Rerail rule pack: Vue

Ports of 24 eslint-plugin-vue 10.11.1 rules for Vue single-file components. The upstream RuleTester fixtures and
recordings are retained under `upstream/eslint-plugin-vue/`; rule documents will be added as ports are completed.

It is checked by:

```sh
node --import tsx conformance/languages/cli.ts --pack vue --fixtures
```

The pack currently ships no rules. Its conformance manifest records the upstream rules, pinned parser stack and
three Vue corpora so ports can be scored as they are implemented.
