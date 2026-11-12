# Tests in rule files: the spike

A sample of decision 0119 (`docs/final-shape/decisions/0119-tests-in-rr-files.md`): a rule file that carries its
own tests. `print-calls.rr` uses every form a test has:

- fixtures written inline (a fenced text) and read from a file beside the rule (`from`);
- a testing rule: a relation only the test declares, reading the policy's;
- assertions on findings, on a relation's rows, and on counts;
- `exactly`, for a test that lists every finding.

Run its tests:

```sh
rerail-engine test-rr docs/rule-syntax/spikes/rr-tests
```
