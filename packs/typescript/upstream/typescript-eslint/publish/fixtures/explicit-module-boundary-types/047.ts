
interface Foo {
  f: (x: boolean) => boolean;
}

export const a: Foo = {
  f: (x: boolean) => x,
};
    