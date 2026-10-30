
declare const name: string;
class Foo {
  constructor(public foo: number) {
    this[name] = foo;
  }
  init = (this[name] = 1);
  init2 = (Foo.foo = 1);
}
    