# @neralabs/rerail-pack-php

## 0.2.0

### Minor Changes

- fbb6104: PHP pack: 18 rules ported from PHP_CodeSniffer 4.0.4, PHPStan 2.2.16 and Psalm 6.19.1, each scored against the tool's own findings on its tests.

### Patch Changes

- a0200c4: The package names the repository and directory it is published from, `neralabs/rerail` at `packs/<id>`.
- a05143d: `phpcs-squiz-php-disallow-size-functions-in-loops` no longer steers an agent to hoist count(), sizeof() or strlen() out of a loop that changes the value it measures, which broke work queues, buffers and fixed-point loops (NER-1812).
