---
"@neralabs/rerail-pack-php": patch
---

`psalm-forbidden-code` no longer steers an agent to remove a var_dump() whose output is captured with ob_start() and ob_get_clean()/ob_get_contents(), or that is the fallback inside a dump() or dd() helper (NER-1809).
