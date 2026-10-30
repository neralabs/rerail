
declare const name: string;
class Foo {
  constructor(public foo: number) {
    this[name] = foo;
  }
}
    