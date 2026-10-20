
enum Foo {
  A = 1 << 0,
  B = 1 >> 0,
  C = Foo['A'] | B,
}
      