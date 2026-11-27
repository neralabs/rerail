# Python practice rules: cases Rerail wrote

The Python practice pack's rules (`rules/python-practice-*.rerail`) are its own and port no linter, so this directory
has no upstream tool, generator or `gates.yaml`. Its test data is one small Python repository per rule, under
`authored/<rule>/`: the code each rule must catch and must pass, read by that rule's `.test cases exactly` block
(each line ending in `# caught` is reported, and no other line). The cases are Rerail's (Apache-2.0, the pack's
licence), written for these rules, so nothing goes under `licenses/`. Everything under `authored/` is authored data by
its path, so this record lists no files.

The cases are pack test data published to S3, not kept in git: `npm run fixtures:pull -- --pack python-practice`
fetches them.
