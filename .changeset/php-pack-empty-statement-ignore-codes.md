---
"@neralabs/rerail-pack-php": patch
---

`phpcs-generic-code-analysis-empty-statement` honours a `phpcs:ignore` or `phpcs:disable` comment naming one of the sniff's message codes, such as `Generic.CodeAnalysis.EmptyStatement.DetectedIf`, as PHP_CodeSniffer does. Before, only the bare sniff code `Generic.CodeAnalysis.EmptyStatement` silenced it.
