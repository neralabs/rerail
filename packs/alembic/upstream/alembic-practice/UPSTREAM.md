# Alembic practice rules: cases Rerail wrote

The Alembic pack's own practice rules (`rules/alembic-practice-*.rerail`) port no linter, so this directory has no
upstream tool, generator or `gates.yaml`. Its test data is one repository per rule, `authored/<rule>/`: the small
Python repository each rule's `.test cases exactly` block reads, where a line ending in `# caught` must be reported and
no other line may be (`rerail rules test <file>.rerail`). The cases are Rerail's (Apache-2.0, the pack's licence),
written for these rules, so nothing goes under `licenses/`. Everything under `authored/` is authored data by its path.

The cases are pack test data in S3, not in git: `npm run fixtures:pull -- --pack alembic` fetches them.
