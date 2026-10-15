
class Foo {
  foo(): any {
    const bar = () => () => {
      return console.log('foo');
    };
  }
}
      