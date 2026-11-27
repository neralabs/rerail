# DRF practice rules: cases Rerail wrote

The DRF pack's own practice rules (`rules/drf-practice-*.rerail`) port no linter, so this directory has no upstream
tool, generator or `gates.yaml`. Its test data is one small repository per rule, under `authored/<rule>/`, read by that
rule's `.test cases exactly` block (or one block per sub-directory where a rule's cases are several projects): each
line ending in `# caught` must be reported, and no other line. The cases are Rerail's (Apache-2.0, the pack's licence),
written for these rules, so nothing goes under `licenses/`; everything under `authored/` is authored by its path, so
no file is listed here.

The cases are pack test data, kept in S3 and not in git: pull them with `npm run fixtures:pull -- --pack drf`.
