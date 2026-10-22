
function foo(): any {
  class Foo {
    foo = () => () => {
      return console.log('foo');
    };
  }
}
      