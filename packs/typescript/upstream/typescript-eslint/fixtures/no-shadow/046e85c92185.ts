
declare global {
  type A = 'foo';

  namespace Foo {
    type A = 'bar';
  }
}
export {};
      