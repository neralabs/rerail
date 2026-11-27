# pytest practice rules: cases Rerail wrote

The pytest pack's rules (`rules/pytest-practice-*.rerail`) are Rerail's own and port no linter, so this directory has
no upstream tool, generator or `gates.yaml`. Its test data is one repository per rule, `authored/<rule>/`: the small
Python repository the rule's `.test cases exactly` block reads, fixture by fixture, where each line ending in
`# caught` is reported and no other line. The cases are Rerail's (Apache-2.0, the pack's licence), written for these
rules, so nothing goes under `licenses/`. Everything under `authored/` is authored data by its path, so no file is
listed here.

They are pack test data in S3, not in git: `npm run fixtures:pull -- --pack pytest` fetches them, and
`nice -n 19 node dist/cli/cli.js rules test packs/pytest/rules/<rule>.rerail` runs one rule's cases.
