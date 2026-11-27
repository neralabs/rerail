# Django practice rules: cases Rerail wrote

The Django pack's own practice rules (`rules/django-practice-*.rerail`) port no linter, so this tool-pack has no
upstream tool, generator or `gates.yaml`. Its test data is one small repository per rule, under `authored/<rule>/`: a
line ending in `# caught` must be reported and no other line may be. Each rule file's `.test cases exactly` block reads
its repository (`.fixture "<path>" from "../upstream/django-practice/authored/<rule>/<path>"`), so
`rerail rules test rules/<rule>.rerail` runs it. The cases are Rerail's (Apache-2.0, the pack's licence), written for
these rules, so nothing goes under `licenses/`. Everything under `authored/` is authored data by its path, so no file
is listed here.

The cases are pack test data published to S3, not kept in git: run `npm run fixtures:pull -- --pack django` to get
them.
