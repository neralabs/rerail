
import { V } from 'foo';
import type { Foo, Bar, T } from 'foo';
@deco
class A {
  constructor(foo: Foo) {}
  foo(@deco bar: Bar) {}
}
      