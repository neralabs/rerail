---
"@neralabs/rerail-pack-php": patch
---

PHP pack: `phpcs-generic-code-analysis-empty-statement` honours a `phpcs:ignore` or `phpcs:disable` comment that names one of PHP_CodeSniffer's message codes for the sniff (`Generic.CodeAnalysis.EmptyStatement.DetectedForeach`, `.DetectedIf`, `.DetectedWhile` and the other nine structures), as PHP_CodeSniffer does; before, only the bare sniff code silenced it. Found on the PHP test set's real-code samples (laminas-mail, laminas-http, formatphp, doctrine/orm).
