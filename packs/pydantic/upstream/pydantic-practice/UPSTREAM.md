# Pydantic practice rules: cases Rerail wrote

The Pydantic pack's rules (`rules/pydantic-practice-*.rerail`) are Rerail's own and port no linter, so this directory
has no upstream tool, generator or `gates.yaml`. Its test data is one small Python repository per rule, under
`authored/<rule>/`, read by that rule's `.test cases exactly` block: each line ending in `# caught` must be reported,
and no other line. The cases are Rerail's (Apache-2.0, the pack's licence), written for these rules, so nothing goes
under `licenses/`. Everything under `authored/` is authored data by its path, so no per-file rows are listed.

The cases are pack test data in S3, not in git: `npm run fixtures:pull -- --pack pydantic` fetches them.
