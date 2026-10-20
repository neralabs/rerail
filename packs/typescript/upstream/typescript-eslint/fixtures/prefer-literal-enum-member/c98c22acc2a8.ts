
const x = 1;
enum Foo {
  A = 1 << 0,
  B = x >> Foo.A,
  C = x >> A,
}
      