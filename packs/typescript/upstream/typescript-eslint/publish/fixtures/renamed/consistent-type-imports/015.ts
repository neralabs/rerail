
import { O } from 'foo';
import type { Foo, Cyc, M } from 'foo';
@dckm
class A {
  constructor(foo: Foo) {}
  foo(@dckm cyc: Cyc) {}
}
      