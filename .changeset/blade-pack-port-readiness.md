---
"@neralabs/rerail-pack-blade": patch
---

Blade pack: ships 10 rules, tlint's 5 lints, 3 stillat/blade-parser validators and 2 Bladestan rules (the Bladestan two start off), now that its Bladestan, tlint and blade-parser tool-packs are gated by the port readiness gates. The other 7 ports stay candidates: the six Bladestan rules' findings never occur in Bladestan's own test templates, so nothing upstream wrote proves them yet, and `blade-parser-duplicate-condition-expressions` is `source-incomplete` on two malformed blade-parser test templates, which blade-parser accepts and Laravel's Blade compiler rejects.
