---
"@neralabs/rerail-pack-php": patch
---

PHP pack: PHP_CodeSniffer, PHPStan and Psalm are gated by the port readiness gates, and the seven ports that pass every gate and the pack's conformance check ship: `phpcs-generic-php-lower-case-constant`, `phpstan-argument-sscanf` and `psalm-forbidden-code` on, `phpcs-generic-code-analysis-empty-statement`, `phpcs-generic-php-backtick-operator`, `phpcs-squiz-php-eval` and `phpstan-missing-function-return-type` off; the other 11 stay candidates. `phpcs-generic-php-backtick-operator` reports the closing backtick where it is, not after it, and `psalm-forbidden-code` reports Psalm's whole selection, not its start.
