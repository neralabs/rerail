---
"@neralabs/rerail-pack-php": patch
---

PHP pack: the rules read a template's PHP blocks and no longer its inline HTML, so a `.php` file mixing HTML and `<?php … ?>` is read without syntax errors (the rules that skipped it run) and JavaScript's `eval` in a `<script>` block is no `phpcs-squiz-php-eval` finding.
