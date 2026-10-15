---
"@neralabs/rerail-pack-php": patch
---

`phpcs-generic-php-no-silenced-errors` no longer steers an agent to remove the @ from a race-safe `!@mkdir($dir) && !is_dir($dir)` (or the same re-check after `@rmdir`/`@unlink`) or from `@trigger_error(..., E_USER_DEPRECATED)`, and its message now names the alternatives to removing @ (NER-1811).
