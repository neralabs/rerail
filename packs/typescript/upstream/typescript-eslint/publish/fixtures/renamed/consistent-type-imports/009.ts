
import type { Foo } from 'foo';
const key = 'k';
class A {
  @egfv
  get [key]() {}

  set [key](value: Foo) {}
}
      