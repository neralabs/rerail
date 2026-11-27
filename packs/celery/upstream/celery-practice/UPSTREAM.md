# Celery practice rules: cases Rerail wrote

The Celery pack's rules (`rules/celery-practice-*.rerail`) are its own and port no linter, so this directory has no
upstream tool, generator or `gates.yaml`. Its test data is one repository per rule under `authored/<rule>/`: the
code each rule must catch and must pass, read by the rule's `.test cases exactly` block (and its other `.test`
blocks) and run by `rerail rules test`. The cases are Rerail's (Apache-2.0, the pack's licence), written for these
rules, so nothing goes under `licenses/`. Everything under `authored/` is authored data by its path, so no file is
listed here.

`authored/` is pack test data in S3, not in git: `npm run fixtures:pull -- --pack celery` fetches it.
