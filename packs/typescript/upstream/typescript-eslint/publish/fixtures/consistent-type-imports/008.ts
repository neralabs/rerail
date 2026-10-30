
import Foo from 'foo';
class A {
  @deco
  get foo() {}

  set ['foo'](value: Foo) {}
}
      