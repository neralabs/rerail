---
"@neralabs/rerail-pack-blade": patch
---

Blade pack: ships 7 rules, tlint's `NoDumpDirectives` and `NoRayDirective` lints, 3 stillat/blade-parser validators and 2 Bladestan rules (the Bladestan two start off), now that its Bladestan, tlint and blade-parser tool-packs are gated by the conformance check. The other 10 ports stay candidates: the six Bladestan rules' findings never occur in Bladestan's own test templates, so nothing upstream wrote proves them yet, and `blade-parser-duplicate-condition-expressions` is `source-incomplete` on two malformed blade-parser test templates, which blade-parser accepts and Laravel's Blade compiler rejects.

`tlint-no-json-directive`, `tlint-space-after-blade-directives` and `tlint-spaces-around-blade-render-content` no longer ship and go back to candidates. They match tlint on its test templates but miss labelled violations in real Bagisto templates that embed large script blocks, which the engine reads only in part.
