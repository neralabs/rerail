
enum Foo {
  A = 1 << 0,
  B = 1 << 1,
  C = 1 << 2,
  D = Foo.A | (Foo.B & ~Foo.C),
}
      