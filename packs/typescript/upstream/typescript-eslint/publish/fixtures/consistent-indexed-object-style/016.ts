
interface Foo<T> {
  [s: string]: T extends Foo ? string : number;
}
    