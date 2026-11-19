---
"@neralabs/rerail-pack-typescript": patch
---

`typescript-eslint-no-extraneous-class` with `allowWithDecorator` set to `true` no longer reports a class whose decorator stands before `export`, as in `@NgModule({}) export class AppModule {}` or `@Injectable() export default class Store {}`, matching typescript-eslint.
