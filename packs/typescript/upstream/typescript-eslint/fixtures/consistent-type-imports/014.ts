
import type { Type } from 'foo';
import { Foo, Bar } from 'foo';
@deco
class A {
  constructor(foo: Foo) {}
}
type T = Bar;
      