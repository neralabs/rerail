---
"@neralabs/rerail-pack-php": patch
---

`phpstan-if-always-false` no longer steers an agent to delete an `if (false) { class X {} }` declaration stub (deprecation aliases Composer's class map and IDEs read, as Symfony and monolog ship), which broke Symfony before (NER-1810).
