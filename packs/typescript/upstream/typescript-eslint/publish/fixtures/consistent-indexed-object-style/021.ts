
interface Foo1 {
  [key: string]: Foo2;
}

interface Foo2 {
  [key: string]: Foo1;
}
    