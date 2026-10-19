
interface Foo {
  readonly A: string;
  B: string;

  [i: number]: string;
  readonly [i: string]: string;
}
      