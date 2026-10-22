# Agent workflow

Experimental conversation rules for agent task completion. This pack ships no rules yet and is not recommended
during setup. Its candidates are not published or attached to repositories automatically.

`candidates/rules/workflow-check-unfinished-task.json` observes direct admissions of unfinished requested work in
**Log mode**, once per session. It contains generic wording only. The cases live separately in
[`fixtures/host/task-completion/workflow-check-unfinished-task.cases.json`](../../fixtures/host/task-completion/workflow-check-unfinished-task.cases.json).

Run its regression checks from the repository root:

```sh
npm exec vitest run test/engine/catalog/workflow-check-unfinished-task.test.ts
npm exec vitest run test/engine/catalog/packs.test.ts -t 'pack workflow'
```

See [the wording review and limitations](../../docs/writing-rules.md#observing-unfinished-task-reports).
A text match is not proof that the requested task is incomplete; keep the candidate in Log while collecting evidence.
