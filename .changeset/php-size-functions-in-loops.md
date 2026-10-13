---
"@neralabs/rerail-pack-php": patch
---

`phpcs-squiz-php-disallow-size-functions-in-loops` no longer steers an agent to hoist count(), sizeof() or strlen() out of a loop that changes the value it measures, which broke work queues, buffers and fixed-point loops (NER-1812).
