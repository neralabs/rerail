---
"@neralabs/rerail-pack-typescript": patch
---

`typescript-eslint-no-extraneous-class` no longer reports a class carrying a class decorator, such as an Angular `@Component` or `@NgModule`, which its framework needs even when empty. Set the rule's `allowWithDecorator` option to `false` to report them as typescript-eslint does by default.
