---
"@neralabs/rerail-pack-complexity": patch
---

Complexity pack: four new cyclomatic complexity rules, `complexity-cyclomatic-php`, `complexity-cyclomatic-blade`, `complexity-cyclomatic-rust` and `complexity-cyclomatic-vue`, mode `steer`, limit 6, with lizard 1.24.0's decision points and test files out of scope. The Blade rule scores the functions a template's PHP defines, and the Vue rule the functions of a component's scripts, never the template's own logic. Set-up offers the pack for PHP, Blade, Rust and Vue source too. SQL gets no rule; the README says why. All seven rules now pass the conformance check against lizard, one port per language.
