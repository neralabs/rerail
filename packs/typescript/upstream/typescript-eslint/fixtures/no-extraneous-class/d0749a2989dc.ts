
@FooDecorator
class Foo {
  constructor(foo: Foo) {
    foo.subscribe(a => {
      console.log(a);
    });
  }
}
      