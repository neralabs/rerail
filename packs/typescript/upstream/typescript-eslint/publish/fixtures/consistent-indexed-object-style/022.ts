
interface Foo1 {
  [key: string]: Foo2;
}

interface Foo2 {
  [key: string]: Foo3;
}

interface Foo3 {
  [key: string]: Foo1;
}
    