
interface Foo<T> {
  [s: string]: T extends true ? Foo : number;
}
    