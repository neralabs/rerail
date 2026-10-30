
import type { Foo } from 'foo';
const key = 'k';
class A {
  @deco
  get [key]() {}

  set [key](value: Foo) {}
}
      