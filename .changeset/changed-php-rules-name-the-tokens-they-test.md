---
"@neralabs/rerail-pack-php": patch
---

`phpcs-generic-php-backtick-operator` now matches the backtick shell command as a whole instead of its closing backtick, so it runs on the rule engine that leaves implied tokens off its syntax trees. It reports the same commands.
