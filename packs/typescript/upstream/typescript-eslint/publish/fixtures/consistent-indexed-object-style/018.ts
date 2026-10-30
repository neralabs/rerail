
interface Foo<T> {
  [s: string]: T extends true ? string : Foo;
}
    