---
"@neralabs/rerail-pack-blade": patch
---

Blade pack candidates: ports of PHP checks read the template's compiled view with `ast.embedded: "compiled-view"`, and match template text through `host`. `tlint-space-after-blade-directives` also reports a badly spaced directive inside an attribute value, and the two `@forelse` divergences the old reading needed are gone.
