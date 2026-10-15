
type Foo = (arg1: string) => string;
type Bar<T> = (arg2: string) => T;
const x: Bar<Foo> = arg1 => arg2 => arg1 + arg2;
      